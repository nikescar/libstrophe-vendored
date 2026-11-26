//! Public-API tests for XMPP over BOSH (XEP-0124 / XEP-0206).
//! Protocol behavior is covered by the mock-server tests in `src/xmpp/bosh/mod.rs`.
#![cfg(feature = "bosh")]

use libstrophe_vendored::xmpp::{bosh::connect_bosh, Error, XmppEvent};

/// `http://` is refused before any network I/O: SASL PLAIN would leak the password.
#[test]
fn plain_http_is_refused() {
    let (_tx, rx) = async_channel::unbounded();
    let r = smol::block_on(connect_bosh("http://localhost:5280/http-bind", "u@localhost", "pw", rx, |_| true));
    assert_eq!(r, Err(Error::Auth));
}

/// Live login: `XMPP_BOSH_URL=https://… XMPP_JID=… XMPP_PASS=…
/// cargo test --features bosh -- --ignored live_bosh_login`
#[test]
#[ignore]
fn live_bosh_login() {
    let url = std::env::var("XMPP_BOSH_URL").expect("XMPP_BOSH_URL");
    let jid = std::env::var("XMPP_JID").expect("XMPP_JID");
    let pass = std::env::var("XMPP_PASS").expect("XMPP_PASS");
    let (_tx, rx) = async_channel::unbounded();
    let mut connected = false;
    let r = smol::block_on(connect_bosh(&url, &jid, &pass, rx, |e| {
        if let XmppEvent::Error(m) = &e {
            eprintln!("error: {m}");
        }
        connected |= e == XmppEvent::Connect;
        false
    }));
    assert_eq!(r, Ok(()));
    assert!(connected);
}
