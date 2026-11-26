# libstrophe-vendored

Vendored [libstrophe](https://github.com/strophe/libstrophe) XMPP client library with **rustls + ring** TLS backend for easy cross-platform compilation.

## Why This Exists

The original libstrophe requires OpenSSL, which is difficult to cross-compile for Android and other platforms. This crate:

- **Vendors the C source** - No system dependencies required
- **Uses rustls + ring** - Pure Rust TLS implementation (no OpenSSL)
- **Compiles via cc crate** - Works anywhere Rust compiles
- **Embeds expat parser** - Self-contained XML parsing
- **Pre-generated bindings** - No libclang required at build time

Perfect for embedding XMPP in Rust mobile apps (Android/iOS), WASM, or any platform where OpenSSL is problematic.

## Features

### TLS Implementation (Complete)

**Phase 1 - Basic Connectivity**
- TLS handshake (`tls_start`, `tls_stop`)
- I/O operations (`tls_read`, `tls_write`, `tls_pending`)
- Error handling (`tls_error`, `tls_is_recoverable`)
- Session lifecycle (`tls_new`, `tls_free`)

**Phase 2 - Certificate Validation**
- Extract XMPP addresses from X.509 certificates (id-on-xmppAddr SAN)
- `tls_id_on_xmppaddr()` - Get XMPP JID from certificate
- `tls_id_on_xmppaddr_num()` - Count XMPP addresses
- Uses x509-parser for DER/ASN.1 parsing

**Phase 3 - Channel Binding**
- tls-server-end-point (SHA-256 hash of server cert)
- `tls_init_channel_binding()` - Initialize for SASL EXTERNAL
- `tls_get_channel_binding_data()` - Get binding hash
- Full SASL EXTERNAL authentication support

### Cargo Features

- `tls-rustls` *(default)* - rustls + ring TLS backend
- `libstrophe-0_14` *(default)* - Latest libstrophe API version
- `buildtime_bindgen` - Regenerate bindings (requires libclang)

## Installation

```toml
[dependencies]
libstrophe-vendored = { git = "https://github.com/nikescar/libstrophe-vendored.git" }
```

## Usage

### Safe API (`xmpp` module)

`libstrophe_vendored::xmpp` wraps the FFI in safe Rust. Owned handles
(`Context`, `Connection`, `Stanza`, `TlsCert`) free their C object on drop and
deref to borrowed views (`ContextRef`, `Conn`, `StanzaRef`, `TlsCertRef`), which
is what callbacks receive. Lifetimes tie connections and stanzas to their
`Context`. Handlers are Rust closures.

```rust,no_run
use libstrophe_vendored::xmpp::{Connection, Context, EventKind, LogLevel, Stanza, NS_ROSTER};

let ctx = Context::with_default_logger(LogLevel::Debug);
let conn = Connection::new(&ctx);
conn.set_jid("user@example.com").unwrap();
conn.set_pass("password").unwrap();

conn.connect_client(None, 0, |conn, event| match event.kind {
    EventKind::Connect => {
        let ctx = conn.context();
        let mut iq = Stanza::iq(ctx, "get", "roster1").unwrap();
        let mut query = Stanza::new(ctx);
        query.set_name("query").unwrap();
        query.set_ns(NS_ROSTER).unwrap();
        iq.add_child(query).unwrap();

        conn.id_handler_add("roster1", |conn, reply| {
            if let Some(query) = reply.child_by_name("query") {
                for item in query.children() {
                    println!("{}", item.attribute("jid").unwrap_or("?"));
                }
            }
            conn.disconnect();
            false // one-shot: unregister
        })
        .unwrap();
        conn.send(&iq);
    }
    _ => conn.context().stop(),
})
.unwrap();
ctx.run();
```

Notes:

- `xmpp_initialize()` runs once on first `Context` creation; `xmpp_shutdown()` is never called.
- Handler closures are freed when their `Connection` is dropped.
- `Conn::handler_delete` disables a handler; libstrophe drops it the next time it would fire.
- A panic inside a callback aborts the process (unwinding into C is not allowed).

### WebSocket API (RFC 7395)

Enable with the `websocket` feature:

```toml
[dependencies]
libstrophe-vendored = { git = "...", features = ["websocket"] }
```

Async WebSocket connection using `smol` runtime:

```rust,no_run
use libstrophe_vendored::xmpp::websocket::{connect_websocket, WsEvent};

smol::block_on(async {
    connect_websocket(
        "wss://example.com:5281/xmpp-websocket",
        "user@example.com",
        "password",
        |event| {
            match event {
                WsEvent::Connect => {
                    println!("Connected!");
                    true // Continue
                }
                WsEvent::Stanza(xml) => {
                    println!("Received: {}", xml);
                    true
                }
                WsEvent::Disconnect => {
                    println!("Disconnected");
                    false // Stop
                }
                WsEvent::Error(msg) => {
                    eprintln!("Error: {}", msg);
                    false
                }
            }
        }
    ).await
});
```

Features:
- **RFC 7395 compliant** - Proper `<open/>` / `<close/>` framing
- **Sub-protocol negotiation** - `Sec-WebSocket-Protocol: xmpp`
- **Async/await** - Built on `smol` runtime
- **TLS support** - `wss://` via `futures-rustls`

See `examples/websocket_basic.rs` for a complete example.

### Examples

Rust ports of the upstream libstrophe C examples, written without `unsafe`:

| Example | Upstream | Shows |
|---------|----------|-------|
| `uuid` | `uuid.c` | UUID generation |
| `basic` | `basic.c` | Connect/disconnect, connection flags |
| `active` | `active.c` | iq + id handler (disco#items sessions) |
| `roster` | `roster.c` | Roster query |
| `component` | `component.c` | External component connection |
| `vcard` | `vcard.c` | Raw send, id + timed handlers, base64 photo |
| `bot` | `bot.c` | Stanza handlers, replies, Stream Management reconnect |
| `complex` | `complex.c` | TLS options, certificate printing, certfail, sockopt |
| `register` | `register.c` | Raw connection, STARTTLS, in-band registration |
| `chat_actor` | (none) | Actor thread owns `Context`; presence broadcast (show/status), echo reply, cross-thread send queue |
| `websocket_basic` | (none) | Async WebSocket connection (RFC 7395) |
| `websocket_bridge` | (none) | WebSocket-to-libstrophe thread bridge with channels |

```bash
cargo run --example basic -- --jid user@example.com --pass password
cargo run --example roster -- user@example.com password
```

Differences from the C versions: password prompts echo input, ports parse as
decimal only, and missing elements print a message instead of crashing.

### Raw FFI

The `ffi` module exposes the C API directly. See
[`examples/ffi_connection.rs`](examples/ffi_connection.rs) for a complete example.

```rust
use libstrophe_vendored::ffi;
use std::ffi::CString;
use std::ptr;

unsafe {
    // Initialize
    ffi::xmpp_initialize();

    // Create context and connection
    let ctx = ffi::xmpp_ctx_new(ptr::null_mut(), ptr::null_mut());
    let conn = ffi::xmpp_conn_new(ctx);

    // Configure connection
    let jid = CString::new("user@example.com").unwrap();
    let pass = CString::new("password").unwrap();
    ffi::xmpp_conn_set_jid(conn, jid.as_ptr());
    ffi::xmpp_conn_set_pass(conn, pass.as_ptr());

    // Enable TLS (mandatory with rustls backend)
    ffi::xmpp_conn_set_flags(conn, 2); // XMPP_CONN_FLAG_MANDATORY_TLS

    // Connect
    extern "C" fn conn_handler(
        _conn: *mut ffi::xmpp_conn_t,
        event: ffi::xmpp_conn_event_t,
        _error: i32,
        _stream_error: *mut ffi::xmpp_stream_error_t,
        _userdata: *mut std::ffi::c_void,
    ) {
        println!("Connection event: {:?}", event);
    }

    ffi::xmpp_connect_client(
        conn,
        ptr::null(), // auto-detect server
        0,           // default port
        Some(conn_handler),
        ptr::null_mut(),
    );

    // Event loop
    for _ in 0..30 {
        ffi::xmpp_run_once(ctx, 1000);
    }

    // Cleanup
    ffi::xmpp_disconnect(conn);
    ffi::xmpp_conn_release(conn);
    ffi::xmpp_ctx_free(ctx);
    ffi::xmpp_shutdown();
}
```

Run the example:

```bash
XMPP_JID="user@example.com" XMPP_PASS="password" \
cargo run --example ffi_connection
```

## Building

### Standard Build

```bash
cargo build --release
```

The C sources are compiled with `-std=gnu11`; GCC 15+ defaults to C23, which
libstrophe's handler code does not compile under.

### Cross-Compilation (Android)

```bash
# Using cargo-ndk
cargo ndk -t arm64-v8a build --release

# Or with zig (cargo-zigbuild)
cargo zigbuild --target aarch64-linux-android --release
```

### WASM

```bash
cargo build --target wasm32-unknown-unknown --release
```

## Testing

```bash
# Unit tests
cargo test

# Integration tests
cargo test --test integration

# Safe API tests (uses a local fake XMPP server, no network needed)
cargo test --test safe_api

# Specific test
cargo test test_tls_backend_lifecycle
```

## Architecture

```
┌─────────────────────────────────────────┐
│          Rust Application               │
└─────────────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│        Safe API (xmpp module)           │
│  (owned/borrowed handles, closures)     │
└─────────────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│          C FFI Layer (ffi.rs)           │
│  (Rust bindings to libstrophe)          │
└─────────────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│       libstrophe C Library              │
│  (XMPP protocol, stanza handling)       │
└─────────────────────────────────────────┘
       │                        │
       ▼                        ▼
┌──────────────┐      ┌──────────────────┐
│ expat (XML)  │      │ TLS Backend      │
│  (vendored)  │      │  (rustls+ring)   │
└──────────────┘      └──────────────────┘
                               │
                               ▼
                      ┌──────────────────┐
                      │ webpki-roots     │
                      │ (Mozilla CAs)    │
                      └──────────────────┘
```

### Directory Structure

```
libstrophe-vendored/
├── c-src/
│   ├── libstrophe/          # Vendored libstrophe C source
│   ├── expat/               # Vendored expat XML parser
│   └── tls_rustls_glue.c    # C accessor functions for Rust TLS
├── src/
│   ├── ffi.rs               # Pre-generated C bindings
│   ├── lib.rs               # Library root (ffi, xmpp, tls_backend)
│   ├── xmpp/                # Safe wrapper (context, connection, stanza, cert)
│   └── tls_backend/
│       ├── session.rs       # TLS session (rustls wrapper)
│       ├── cert.rs          # Certificate parsing (x509-parser)
│       └── ffi.rs           # C FFI exports for TLS
├── build.rs                 # Compile C sources via cc crate
├── examples/                # Safe ports of upstream examples + ffi_connection
└── tests/                   # Smoke, integration, safe API tests
```

## Dependencies

### Rust Crates

- **rustls** 0.23 - Pure Rust TLS implementation
- **ring** - Cryptographic primitives (via rustls)
- **webpki-roots** 0.26 - Mozilla CA certificate bundle
- **x509-parser** 0.16 - X.509 certificate parsing
- **sha2** 0.10 - SHA-256 for channel binding
- **cc** 1.0 - Compile C source at build time

### C Source (Vendored)

- **libstrophe** master (0.14.0-26-gf549850) - XMPP client library (MIT OR GPL-3.0, using MIT)
- **expat** 2.6.4 - XML parser (MIT)

## Security

### Certificate Validation

- Uses webpki-roots (Mozilla's CA bundle)
- Validates hostname against certificate SANs
- Extracts XMPP addresses (id-on-xmppAddr) per RFC 6120
- Rejects expired/invalid certificates

### Channel Binding

- Implements tls-server-end-point (RFC 5929)
- SHA-256 hash of server certificate
- Prevents MitM attacks with SASL EXTERNAL

### No OpenSSL Dependencies

- Pure Rust cryptography (ring)
- No unsafe memory management bugs
- No runtime linking issues
- Easier security auditing

## License

MIT License - see [LICENSE](LICENSE) file for details.

This project vendors:
- **libstrophe** - Dual-licensed MIT OR GPL-3.0, we use it under MIT
- **expat** - MIT License

## Documentation

- [libstrophe C API Documentation](https://strophe.im/libstrophe/)
- [libstrophe GitHub](https://github.com/strophe/libstrophe)
- [XMPP RFC 6120](https://www.rfc-editor.org/rfc/rfc6120)
