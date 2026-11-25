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

✅ **Phase 1 - Basic Connectivity**
- TLS handshake (`tls_start`, `tls_stop`)
- I/O operations (`tls_read`, `tls_write`, `tls_pending`)
- Error handling (`tls_error`, `tls_is_recoverable`)
- Session lifecycle (`tls_new`, `tls_free`)

✅ **Phase 2 - Certificate Validation**
- Extract XMPP addresses from X.509 certificates (id-on-xmppAddr SAN)
- `tls_id_on_xmppaddr()` - Get XMPP JID from certificate
- `tls_id_on_xmppaddr_num()` - Count XMPP addresses
- Uses x509-parser for DER/ASN.1 parsing

✅ **Phase 3 - Channel Binding**
- tls-server-end-point (SHA-256 hash of server cert)
- `tls_init_channel_binding()` - Initialize for SASL EXTERNAL
- `tls_get_channel_binding_data()` - Get binding hash
- Full SASL EXTERNAL authentication support

### Cargo Features

- `tls-rustls` *(default)* - rustls + ring TLS backend
- `tls-dummy` - No TLS support (testing only)
- `libstrophe-0_14` *(default)* - Latest libstrophe API version
- `buildtime_bindgen` - Regenerate bindings (requires libclang)

## Installation

```toml
[dependencies]
libstrophe-vendored = { git = "https://github.com/nikescar/libstrophe-vendored.git" }
```

## Usage

This crate provides low-level FFI bindings to libstrophe. See [`examples/ffi_connection.rs`](examples/ffi_connection.rs) for a complete example.

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

# Specific test
cargo test test_tls_backend_lifecycle
```

**Test coverage:** 15/15 tests passing (11 unit + 4 integration)

## Architecture

```
┌─────────────────────────────────────────┐
│          Rust Application               │
│           (FFI bindings)                │
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
│   ├── lib.rs               # Library root (re-exports ffi + tls_backend)
│   └── tls_backend/
│       ├── session.rs       # TLS session (rustls wrapper)
│       ├── cert.rs          # Certificate parsing (x509-parser)
│       └── ffi.rs           # C FFI exports for TLS
├── build.rs                 # Compile C sources via cc crate
├── examples/                # Usage examples
└── tests/                   # Integration tests
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

- **libstrophe** 0.14 - XMPP client library (MIT OR GPL-3.0, using MIT)
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
