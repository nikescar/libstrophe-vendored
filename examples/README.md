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

## Testing Without a Server

See `tests/integration.rs` for API tests that don't require a live server.

```bash
cargo test --test integration
```
