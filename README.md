<h1 align="center">libstrophe-vendored</h1>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=for-the-badge" alt="License: MIT"></a>
  <a href="VERSION"><img src="https://img.shields.io/badge/libstrophe-0.14.0--26-blue?style=for-the-badge" alt="libstrophe 0.14.0-26"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/Rust-1.75%2B-orange?style=for-the-badge&logo=rust&logoColor=white" alt="Rust 1.75+"></a>
  <a href="#tls"><img src="https://img.shields.io/badge/TLS-rustls%20%2B%20ring-blueviolet?style=for-the-badge" alt="TLS: rustls + ring"></a>
</p>

**The [libstrophe](https://github.com/strophe/libstrophe) XMPP client library, vendored and built from source with a pure-Rust TLS backend: no OpenSSL, no system libraries, no libclang.**

Upstream libstrophe needs OpenSSL or GnuTLS, which is painful to cross-compile for Android and other targets. This crate compiles libstrophe and expat with the `cc` crate, replaces the TLS layer with rustls + ring, ships pre-generated bindings, and adds a safe Rust API on top, plus async WebSocket and BOSH transports that libstrophe does not have.

<table>
<tr><td><b>No system dependencies</b></td><td>libstrophe and expat are vendored C sources compiled by <code>build.rs</code>. TLS is rustls + ring with Mozilla roots (<code>webpki-roots</code>). Bindings are pre-generated, so no libclang at build time.</td></tr>
<tr><td><b>Safe Rust API</b></td><td><code>xmpp::Context</code>, <code>Connection</code>, <code>Stanza</code>: owned handles free their C object on drop, callbacks are closures, lifetimes tie connections and stanzas to their context. No <code>unsafe</code> in user code.</td></tr>
<tr><td><b>Four ways to connect</b></td><td>TCP with STARTTLS (default), direct TLS on 5223, WebSocket (RFC 7395) and BOSH (XEP-0124 / XEP-0206). See <a href="#connection-methods">Connection Methods</a>.</td></tr>
<tr><td><b>Modern authentication</b></td><td>SCRAM-SHA-512/256/1 including the <code>-PLUS</code> variants with channel binding, DIGEST-MD5, PLAIN, ANONYMOUS, and Stream Management (XEP-0198) with session resumption.</td></tr>
<tr><td><b>Ports of every upstream example</b></td><td>The nine libstrophe C examples rewritten in safe Rust, plus WebSocket, BOSH and actor-thread examples. All read <code>XMPP_JID</code> / <code>XMPP_PASS</code> from the environment.</td></tr>
<tr><td><b>Tested without a server</b></td><td>Scripted local mock servers cover login flows, STARTTLS/TLS handshakes, BOSH retries and HTTP framing. Live tests run on demand with <code>--ignored</code>.</td></tr>
</table>

---

## Quick Install

```toml
[dependencies]
libstrophe-vendored = { git = "https://github.com/nikescar/libstrophe-vendored.git" }

# async transports are opt-in
libstrophe-vendored = { git = "https://github.com/nikescar/libstrophe-vendored.git", features = ["websocket", "bosh"] }
```

### Cargo Features

| Feature | Default | What it enables |
|---------|---------|-----------------|
| `tls-rustls` | yes | rustls + ring TLS backend (the only backend; required) |
| `libstrophe-0_14` | yes | Latest libstrophe API surface |
| `websocket` | | `xmpp::websocket`: async XMPP over WebSocket (RFC 7395) |
| `bosh` | | `xmpp::bosh`: async XMPP over BOSH (XEP-0124 / XEP-0206) |
| `buildtime_bindgen` | | Regenerate `src/ffi.rs` at build time (needs libclang) |

---

## Getting Started

```bash
export XMPP_JID=user@example.com XMPP_PASS=secret

cargo run --example basic                       # connect, authenticate, disconnect
cargo run --example basic -- --mandatory-tls    # refuse to continue without TLS
cargo run --example roster                      # print your roster
cargo run --example bot                         # echo bot with Stream Management reconnect

XMPP_WS_URL=wss://example.com/xmpp-websocket \
  cargo run --features websocket --example websocket_basic

XMPP_BOSH_URL=https://example.com/http-bind \
  cargo run --features bosh --example bosh_echo
```

Minimal client with the safe API:

```rust,no_run
use libstrophe_vendored::xmpp::{ConnFlags, Connection, Context, EventKind, LogLevel};

let ctx = Context::with_default_logger(LogLevel::Info);
let conn = Connection::new(&ctx);
conn.set_jid("user@example.com").unwrap();
conn.set_pass("secret").unwrap();
conn.set_flags(ConnFlags::MANDATORY_TLS).unwrap(); // never send the password unencrypted

conn.connect_client(None, 0, |conn, event| match event.kind {
    EventKind::Connect => {
        println!("bound as {}", conn.bound_jid().unwrap_or_default());
        conn.disconnect();
    }
    _ => conn.context().stop(),
})
.unwrap();
ctx.run();
```

---

## Connection Methods

| Method | API | Discovery / endpoint | Encryption | Authentication |
|--------|-----|----------------------|------------|----------------|
| **TCP + STARTTLS** (RFC 6120 §5) | `Connection::connect_client` | `_xmpp-client._tcp` SRV, else domain:5222 | Upgraded with `<starttls/>` when offered | All SASL mechanisms below |
| **Direct TLS** (XEP-0368, partial) | `connect_client` + `ConnFlags::LEGACY_SSL` | domain:5223, no SRV lookup | TLS before any XML | All SASL mechanisms below |
| **WebSocket** (RFC 7395) | `xmpp::websocket::connect_websocket` | `wss://` URL | Verified TLS; `ws://` refused | SASL PLAIN |
| **BOSH** (XEP-0124 / XEP-0206) | `xmpp::bosh::connect_bosh` | `https://` URL | Verified TLS; `http://` refused | SASL PLAIN |
| **Component** (XEP-0114) | `Connection::connect_component` | host:port given | None (local trusted link) | SHA-1 handshake |
| **Raw** | `Connection::connect_raw` | as `connect_client` | `Conn::tls_start` on demand | Application-defined |

> **Heads up:** with default flags, STARTTLS is opportunistic. If the server does not offer `<starttls/>` (or an attacker strips it), libstrophe authenticates over plain TCP, and SASL PLAIN then sends the password readable. Set `ConnFlags::MANDATORY_TLS` on every connection that uses a password.

The WebSocket and BOSH clients share one event type, `XmppEvent { Connect, Stanza, Disconnect, Error }`. BOSH also takes an `async_channel::Receiver<String>` for outbound stanzas:

```rust,no_run
use libstrophe_vendored::xmpp::{bosh::connect_bosh, XmppEvent};

let (tx, rx) = async_channel::unbounded::<String>();
smol::block_on(async {
    connect_bosh("https://example.com/http-bind", "user@example.com", "secret", rx, |event| {
        match event {
            XmppEvent::Connect => tx.try_send("<presence/>".into()).is_ok(),
            XmppEvent::Stanza(xml) => { println!("{xml}"); true }
            XmppEvent::Disconnect | XmppEvent::Error(_) => false,
        }
    })
    .await
});
```

The BOSH client keeps one long-poll open for server push and a second request for sends (`hold=1`, `requests=2`), processes responses in `rid` order, resends a lost request with the same `rid` up to 3 times, and ends with `type='terminate'` plus unavailable presence. Its HTTP/1.1 client is hand-written: keep-alive, `Content-Length` and chunked bodies; no redirects, proxies, cookies or compression.

---

## XEP Support

Status:
**Built-in**: the library speaks the protocol itself.
**App-level**: namespace constants and stanza builders are provided; your code sends and handles the stanzas (an example shows how).
**Partial**: works with stated limits.
**Unavailable**: not functional in this build.

| XEP | Name | Status | Details |
|-----|------|--------|---------|
| [XEP-0030](https://xmpp.org/extensions/xep-0030.html) | Service Discovery | App-level | `NS_DISCO_INFO`, `NS_DISCO_ITEMS`. libstrophe does not answer disco queries for you. The `active` example queries `disco#items`. |
| [XEP-0054](https://xmpp.org/extensions/xep-0054.html) | vcard-temp | App-level | The `vcard` example fetches a vCard with an id handler and decodes the base64 photo. |
| [XEP-0077](https://xmpp.org/extensions/xep-0077.html) | In-Band Registration | App-level | `NS_REGISTER`. The `register` example uses a raw connection, STARTTLS via `Conn::tls_start`, then the registration form. |
| [XEP-0078](https://xmpp.org/extensions/xep-0078.html) | Non-SASL Authentication | Built-in, opt-in | `ConnFlags::LEGACY_AUTH`. `jabber:iq:auth` is used only when the server offers no usable SASL mechanism. Obsolete; for very old servers. |
| [XEP-0114](https://xmpp.org/extensions/xep-0114.html) | Jabber Component Protocol | Built-in | `Connection::connect_component(server, port)` with the SHA-1 `<handshake/>`. See the `component` example. |
| [XEP-0124](https://xmpp.org/extensions/xep-0124.html) | BOSH | Built-in (Rust, `bosh`) | Session creation, long polling, `rid`/`sid` tracking, in-order processing, same-`rid` retry, terminate. |
| [XEP-0138](https://xmpp.org/extensions/xep-0138.html) | Stream Compression | Unavailable | This build compiles `compression_dummy.c` (no zlib), so `ConnFlags::ENABLE_COMPRESSION` has no effect. |
| [XEP-0175](https://xmpp.org/extensions/xep-0175.html) | SASL ANONYMOUS | Built-in | Used when the JID has no local part (`example.com`) and the server offers ANONYMOUS. |
| [XEP-0178](https://xmpp.org/extensions/xep-0178.html) | SASL EXTERNAL with certificates | Unavailable | libstrophe advertises EXTERNAL when `Conn::set_client_cert` is used, but the rustls backend builds its config with `with_no_client_auth()`, so no client certificate is presented. |
| [XEP-0198](https://xmpp.org/extensions/xep-0198.html) | Stream Management | Built-in | `<enable resume='true'/>`, `<r/>`/`<a/>` acknowledgements, `<resume/>` on reconnect. Move state to a new connection with `Conn::take_sm_state` / `set_sm_state`; inspect unacked stanzas with `send_queue_len`. Disable with `ConnFlags::DISABLE_SM`. The `bot` example reconnects with resumption. |
| [XEP-0206](https://xmpp.org/extensions/xep-0206.html) | XMPP Over BOSH | Built-in (Rust, `bosh`) | `xmpp:version`, `xmpp:restart='true'` after SASL, resource binding, `jabber:client` namespace added to bare stanzas. |
| [XEP-0368](https://xmpp.org/extensions/xep-0368.html) | SRV records for XMPP over TLS | Partial | `ConnFlags::LEGACY_SSL` (alias `XMPP_CONN_FLAG_DIRECT_TLS`) starts TLS immediately on domain:5223 with SNI set to the JID domain and skips STARTTLS. No `_xmpps-client._tcp` SRV lookup and no ALPN yet. |

Core protocols:

| Spec | Status | Details |
|------|--------|---------|
| RFC 6120 (XMPP Core) | Built-in | Streams, STARTTLS, SASL, resource binding, stream errors (`StreamError`), SRV lookup. Legacy session IQ (`NS_SESSION`) when the server requires it. |
| RFC 6121 (IM and Presence) | App-level | `NS_ROSTER`, `Stanza::message`, `Stanza::presence`, `Stanza::reply`. See the `roster`, `bot` and `chat_actor` examples. |
| RFC 7395 (XMPP over WebSocket) | Built-in (Rust, `websocket`) | `<open/>`/`<close/>` framing, `Sec-WebSocket-Protocol: xmpp`, stream restart after SASL. |

### SASL Mechanisms

| Mechanism | TCP (libstrophe) | WebSocket / BOSH |
|-----------|------------------|------------------|
| SCRAM-SHA-512, SCRAM-SHA-256, SCRAM-SHA-1 | yes | |
| SCRAM-SHA-*-PLUS (channel binding) | yes, `tls-server-end-point` | |
| DIGEST-MD5 | yes | |
| PLAIN | yes, only when no stronger mechanism is offered | yes, TLS only |
| ANONYMOUS | yes | |
| EXTERNAL | no (see XEP-0178) | |

<a id="tls"></a>
### TLS

The rustls backend (`src/tls_backend/`) implements libstrophe's `tls.h` interface: handshake, non-blocking I/O, `id-on-xmppAddr` extraction from certificates (`x509-parser`), and `tls-server-end-point` channel binding (SHA-256 of the server certificate). Server certificates are verified against Mozilla roots for the JID domain.

Limits of this backend: `ConnFlags::TRUST_TLS` and `Conn::set_certfail_handler` are not consulted, so an unverifiable certificate always ends the connection; `set_cafile` / `set_capath` and client certificates are not loaded.

---

## Architecture

```
                           Rust application
             ┌───────────────────┴────────────────────┐
             ▼                                        ▼
┌───────────────────────────────┐   ┌──────────────────────────────────────┐
│ Safe API  (src/xmpp/)         │   │ Async transports  (smol, opt-in)     │
│ Context · Connection · Conn   │   │ xmpp::websocket      xmpp::bosh      │
│ Stanza · TlsCert · SmState    │   │ connect_websocket    connect_bosh    │
│ closures, owned/borrowed      │   ├──────────────────────────────────────┤
└───────────────┬───────────────┘   │ shared:  event.rs  XmppEvent         │
                ▼                   │          auth.rs   SASL PLAIN, bind  │
┌───────────────────────────────┐   │          net.rs    URL, TCP / TLS    │
│ FFI bindings  (src/ffi.rs)    │   ├──────────────────┬───────────────────┤
└───────────────┬───────────────┘   │ async-tungstenite│ HTTP/1.1 client   │
                ▼                   │ (RFC 6455)       │ bosh/http.rs      │
┌───────────────────────────────┐   └─────────┬────────┴─────────┬─────────┘
│ libstrophe C  (c-src/)        │             │                  │
│ event loop · SASL · SM        │             ▼                  ▼
│ STARTTLS · SRV resolver       │   ┌──────────────────────────────────────┐
└──────┬─────────────────┬──────┘   │ futures-rustls                       │
       ▼                 ▼          └──────────────────┬───────────────────┘
┌────────────┐  ┌──────────────────────────┐           │
│ expat      │  │ TLS backend (Rust)       │           │
│ XML parser │  │ src/tls_backend/ via     │           │
│ (vendored) │  │ c-src/tls_rustls_glue.c  │           │
└────────────┘  └────────────┬─────────────┘           │
                             ▼                         ▼
                ┌────────────────────────────────────────────────┐
                │ rustls 0.23 + ring  ·  webpki-roots (Mozilla)  │
                └────────────────────────────────────────────────┘
```

Two independent stacks share one TLS library and trust store:

- **libstrophe stack** (always built). libstrophe owns the socket, the XML stream and the event loop (`Context::run`). It calls back into Rust for TLS through the `tls.h` functions exported by `src/tls_backend/ffi.rs`. `tls_rustls_glue.c` gives Rust access to fields of the opaque `xmpp_conn_t`.
- **Async stack** (`websocket` / `bosh` features). Pure Rust on the smol runtime; libstrophe is not involved. Login runs through a sans-I/O state machine (`auth.rs`) that both transports drive. To use it alongside a libstrophe event loop, run it on its own thread and bridge with channels (`examples/websocket_bridge.rs`).

### Directory Structure

```
libstrophe-vendored/
├── c-src/
│   ├── libstrophe/            # Vendored libstrophe C source (see VERSION)
│   ├── expat/                 # Vendored expat XML parser
│   └── tls_rustls_glue.c      # Accessors into xmpp_conn_t for the Rust TLS backend
├── src/
│   ├── lib.rs                 # Crate root: ffi, xmpp, tls_backend
│   ├── ffi.rs                 # Pre-generated libstrophe bindings
│   ├── tls_backend/           # rustls implementation of libstrophe's tls.h
│   │   ├── session.rs         #   handshake, I/O, channel binding
│   │   ├── cert.rs            #   X.509 parsing, id-on-xmppAddr
│   │   └── ffi.rs             #   extern "C" exports
│   └── xmpp/                  # Safe API
│       ├── ctx.rs conn.rs stanza.rs cert.rs error.rs
│       ├── event.rs           #   XmppEvent (async transports)
│       ├── auth.rs            #   sans-I/O SASL PLAIN / restart / bind
│       ├── net.rs             #   URL parsing, TCP / TLS transport
│       ├── websocket.rs       #   RFC 7395 client
│       └── bosh/              #   XEP-0124 / XEP-0206 client
│           ├── mod.rs         #     session loop, scheduling, retry
│           ├── body.rs        #     <body/> builders and parser
│           └── http.rs        #     HTTP/1.1 POST client
├── examples/                  # Safe ports of upstream examples + transports
├── tests/                     # smoke, integration, safe_api, websocket, bosh
└── build.rs                   # Compiles the C sources with cc
```

---

## Examples

Rust ports of the upstream libstrophe C examples, written without `unsafe`, plus examples for the added transports. All accept `XMPP_JID`, `XMPP_PASS`, `XMPP_HOST`, `XMPP_PORT`, `XMPP_WS_URL` and `XMPP_BOSH_URL` from the environment (environment wins over CLI arguments).

| Example | Upstream | Shows |
|---------|----------|-------|
| `uuid` | `uuid.c` | UUID generation |
| `basic` | `basic.c` | Connect/disconnect, connection flags (`--mandatory-tls`, `--legacy-ssl`, ...) |
| `active` | `active.c` | iq + id handler (disco#items) |
| `roster` | `roster.c` | Roster query |
| `component` | `component.c` | External component connection (XEP-0114) |
| `vcard` | `vcard.c` | Raw send, id + timed handlers, base64 photo |
| `bot` | `bot.c` | Stanza handlers, replies, Stream Management reconnect |
| `complex` | `complex.c` | TLS options, certificate printing, certfail, sockopt |
| `register` | `register.c` | Raw connection, STARTTLS, in-band registration |
| `chat_actor` | | Actor thread owns `Context`; presence broadcast, echo reply, cross-thread send queue |
| `ffi_connection` | | The raw `ffi` module, no safe wrapper |
| `websocket_basic` | | WebSocket connection (`websocket` feature) |
| `websocket_bridge` | | WebSocket thread bridged to a libstrophe event loop |
| `bosh_basic` | | BOSH connect and terminate (`bosh` feature) |
| `bosh_echo` | | BOSH echo bot using the outbound channel |

Differences from the C versions: password prompts echo input, ports parse as decimal only, and missing elements print a message instead of crashing. See [`examples/README.md`](examples/README.md) for per-example usage.

### Raw FFI

The `ffi` module exposes the C API directly for anything the safe API does not cover. [`examples/ffi_connection.rs`](examples/ffi_connection.rs) is a complete client:

```bash
XMPP_JID=user@example.com XMPP_PASS=secret cargo run --example ffi_connection
```

Safe API notes:

- `xmpp_initialize()` runs once on first `Context` creation; `xmpp_shutdown()` is never called.
- Handler closures are freed when their `Connection` is dropped.
- `Conn::handler_delete` disables a handler; libstrophe drops it the next time it would fire.
- A panic inside a callback aborts the process (unwinding into C is not allowed).
- libstrophe is single-threaded: none of the safe types are `Send`/`Sync`.

---

## Building

```bash
cargo build --release

# Android (cargo-ndk or cargo-zigbuild)
cargo ndk -t arm64-v8a build --release
cargo zigbuild --target aarch64-linux-android --release
```

The C sources are compiled with `-std=gnu11`; GCC 15+ defaults to C23, which libstrophe's handler code does not compile under.

## Testing

```bash
cargo test                                   # libstrophe stack: unit, integration, safe_api (local fake servers)
cargo test --features websocket,bosh         # + WebSocket and BOSH mock-server tests

# live tests against a real server (ignored by default)
XMPP_WS_URL=wss://… XMPP_JID=… XMPP_PASS=… cargo test --features websocket -- --ignored live_wss_login
XMPP_BOSH_URL=https://… XMPP_JID=… XMPP_PASS=… cargo test --features bosh -- --ignored live_bosh_login
```

## Dependencies

| Crate | Used for |
|-------|----------|
| `rustls` 0.23 + `ring` | TLS for both stacks |
| `webpki-roots` 0.26 | Mozilla CA bundle |
| `x509-parser` 0.16 | Certificate parsing, `id-on-xmppAddr` |
| `sha2` 0.10 | `tls-server-end-point` channel binding |
| `cc` 1.0 | Compiling the C sources |
| `smol`, `futures-util`, `futures-rustls`, `async-channel`, `base64` | Async transports (`websocket` / `bosh`) |
| `async-tungstenite` 0.28 | WebSocket framing (`websocket`) |

Vendored C: libstrophe master `0.14.0-26-gf549850` (MIT OR GPL-3.0, used under MIT) and expat 2.6.4 (MIT). Exact commits and local modifications are listed in [`VERSION`](VERSION).

---

## Contributing

Issues and pull requests are welcome. Run `cargo test --features websocket,bosh` and `cargo clippy --all-targets --features websocket,bosh` before submitting. Changes to `c-src/libstrophe/` diverge from upstream: record them in `VERSION`.

## License

MIT — see [LICENSE](LICENSE). libstrophe is dual-licensed MIT OR GPL-3.0 and used here under MIT ([LICENSE-LIBSTROPHE](LICENSE-LIBSTROPHE)); expat is MIT ([LICENSE-EXPAT](LICENSE-EXPAT)).
