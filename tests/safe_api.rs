//! Tests for the safe `xmpp` wrapper.
#![forbid(unsafe_code)]

use libstrophe_vendored::xmpp::*;
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- context

#[test]
fn uuid_gen_returns_uuid() {
    let ctx = Context::new();
    let uuid = ctx.uuid_gen().expect("uuid");
    assert_eq!(uuid.len(), 36);
    assert_eq!(uuid.matches('-').count(), 4);
}

#[test]
fn jid_helpers() {
    let ctx = Context::with_default_logger(LogLevel::Error);
    let jid = "user@example.com/res";
    assert_eq!(ctx.jid_bare(jid).as_deref(), Some("user@example.com"));
    assert_eq!(ctx.jid_node(jid).as_deref(), Some("user"));
    assert_eq!(ctx.jid_domain(jid).as_deref(), Some("example.com"));
    assert_eq!(ctx.jid_resource(jid).as_deref(), Some("res"));
    assert_eq!(ctx.jid_domain("example.com").as_deref(), Some("example.com"));
    assert_eq!(ctx.jid_node("example.com"), None);
    assert_eq!(
        ctx.jid_new(Some("u"), "d", Some("r")).as_deref(),
        Some("u@d/r")
    );
}

#[test]
fn base64_roundtrip() {
    let ctx = Context::new();
    assert_eq!(ctx.base64_encode(b"hello").as_deref(), Some("aGVsbG8="));
    assert_eq!(ctx.base64_decode("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
}

#[test]
fn namespace_constants_match_ffi() {
    use libstrophe_vendored::ffi;
    let strip = |b: &[u8]| std::str::from_utf8(&b[..b.len() - 1]).unwrap().to_owned();
    assert_eq!(NS_ROSTER, strip(ffi::XMPP_NS_ROSTER));
    assert_eq!(NS_DISCO_ITEMS, strip(ffi::XMPP_NS_DISCO_ITEMS));
    assert_eq!(NS_REGISTER, strip(ffi::XMPP_NS_REGISTER));
    assert_eq!(NS_TLS, strip(ffi::XMPP_NS_TLS));
    assert_eq!(NS_STREAMS, strip(ffi::XMPP_NS_STREAMS));
}

// ---------------------------------------------------------------- stanza

#[test]
fn build_iq_with_query_child() -> Result<()> {
    let ctx = Context::new();
    let mut iq = Stanza::iq(&ctx, "get", "roster1")?;
    let mut query = Stanza::new(&ctx);
    query.set_name("query")?;
    query.set_ns(NS_ROSTER)?;
    query.set_attribute("node", "sessions")?;
    iq.set_to("example.com")?;
    iq.add_child(query)?;

    assert_eq!(iq.name(), Some("iq"));
    assert_eq!(iq.stanza_type(), Some("get"));
    assert_eq!(iq.id(), Some("roster1"));
    assert_eq!(iq.to(), Some("example.com"));
    let q = iq.child_by_name("query").expect("query child");
    assert_eq!(q.ns(), Some(NS_ROSTER));
    assert_eq!(q.attribute("node"), Some("sessions"));
    assert!(iq.child_by_ns(NS_ROSTER).is_some());
    assert!(iq.child_by_name_and_ns("query", NS_ROSTER).is_some());

    let xml = iq.to_text()?;
    assert!(xml.starts_with("<iq"), "{xml}");
    assert!(xml.contains("jabber:iq:roster"), "{xml}");
    Ok(())
}

#[test]
fn parse_and_iterate_children() -> Result<()> {
    let ctx = Context::new();
    let s = Stanza::from_str(
        &ctx,
        "<iq type='result' id='r1'><query xmlns='jabber:iq:roster'>\
         <item jid='a@b' name='A' subscription='both'/>\
         <item jid='c@d' subscription='to'/></query></iq>",
    )?;
    let query = s.child_by_name("query").unwrap();
    let jids: Vec<_> = query.children().map(|i| i.attribute("jid").unwrap()).collect();
    assert_eq!(jids, ["a@b", "c@d"]);
    let first = query.children().next().unwrap();
    assert_eq!(first.attribute("name"), Some("A"));
    assert_eq!(first.next().unwrap().attribute("name"), None);
    assert!(first.is_tag() && !first.is_text());
    Ok(())
}

#[test]
fn text_children_and_get_text() -> Result<()> {
    let ctx = Context::new();
    let mut name = Stanza::new(&ctx);
    name.set_name("name")?;
    let mut text = Stanza::new(&ctx);
    text.set_text("example bot")?;
    name.add_child(text)?;
    assert_eq!(name.text().as_deref(), Some("example bot"));
    assert!(name.children().next().unwrap().is_text());
    Ok(())
}

#[test]
fn reply_swaps_addresses_and_sets_body() -> Result<()> {
    let ctx = Context::new();
    let msg = Stanza::from_str(
        &ctx,
        "<message from='a@x/r' to='b@y' type='chat' id='m1'><body>hi</body></message>",
    )?;
    assert_eq!(msg.body().as_deref(), Some("hi"));
    let mut reply = msg.reply()?;
    assert_eq!(reply.to(), Some("a@x/r"));
    assert_eq!(reply.id(), Some("m1"));
    reply.set_body("hi to you too!")?;
    assert_eq!(reply.body().as_deref(), Some("hi to you too!"));
    Ok(())
}

#[test]
fn constructors() -> Result<()> {
    let ctx = Context::new();
    assert_eq!(Stanza::presence(&ctx)?.name(), Some("presence"));
    let m = Stanza::message(&ctx, Some("chat"), "to@x", None)?;
    assert_eq!((m.name(), m.stanza_type(), m.to()), (Some("message"), Some("chat"), Some("to@x")));
    let copy = m.copy()?;
    assert_eq!(copy.to(), Some("to@x"));
    Ok(())
}

#[test]
fn add_child_rejects_foreign_context() {
    let a = Context::new();
    let b = Context::new();
    let mut parent = Stanza::iq(&a, "get", "x").unwrap();
    let child = Stanza::iq(&b, "get", "y").unwrap();
    assert_eq!(parent.add_child(child), Err(Error::InvalidOp));
}

#[test]
fn interior_nul_is_rejected() {
    let ctx = Context::new();
    let mut s = Stanza::new(&ctx);
    assert_eq!(s.set_name("a\0b"), Err(Error::Nul));
    assert!(Stanza::from_str(&ctx, "<a\0/>").is_err());
}

#[test]
fn invalid_xml_is_error() {
    let ctx = Context::new();
    assert!(Stanza::from_str(&ctx, "<not closed").is_err());
}

// ---------------------------------------------------------------- connection

#[test]
fn connection_settings_roundtrip() -> Result<()> {
    let ctx = Context::new();
    let conn = Connection::new(&ctx);
    conn.set_jid("u@example.com")?;
    conn.set_pass("secret")?;
    conn.set_flags(ConnFlags::DISABLE_TLS | ConnFlags::LEGACY_AUTH)?;
    assert_eq!(conn.jid(), Some("u@example.com"));
    assert_eq!(conn.pass(), Some("secret"));
    assert!(conn.flags().contains(ConnFlags::DISABLE_TLS));
    assert!(!conn.flags().contains(ConnFlags::MANDATORY_TLS));
    assert!(conn.is_disconnected());
    assert!(!conn.is_secured());
    assert!(conn.peer_cert().is_none());
    assert!(conn.take_sm_state().is_none());
    Ok(())
}

/// Fake XMPP server: accepts one connection, optionally replies with `script`
/// after the client sends its stream header, then waits for the client to close.
fn fake_server(script: Option<String>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut buf = [0u8; 4096];
        if let Some(script) = script {
            let mut seen = Vec::new();
            while !String::from_utf8_lossy(&seen).contains("<stream:stream") {
                match sock.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                }
            }
            sock.write_all(script.as_bytes()).unwrap();
        }
        while matches!(sock.read(&mut buf), Ok(n) if n > 0) {}
    });
    port
}

const STREAM_HEADER: &str = "<?xml version='1.0'?><stream:stream xmlns='jabber:client' \
     xmlns:stream='http://etherx.jabber.org/streams' id='s1' from='localhost' version='1.0'>";

/// Run the event loop until `done` or the deadline (guards against hangs).
fn run_until(ctx: &Context, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "event loop timed out");
        ctx.run_once(Duration::from_millis(10));
    }
}

fn raw_conn(ctx: &Context) -> Connection<'_> {
    let conn = Connection::new(ctx);
    conn.set_jid("localhost").unwrap();
    conn.set_flags(ConnFlags::DISABLE_TLS).unwrap();
    conn
}

#[test]
fn raw_connect_then_disconnect_and_stop() {
    let port = fake_server(None);
    let ctx = Context::new();
    let conn = raw_conn(&ctx);
    let events = Rc::new(RefCell::new(Vec::new()));
    let ev = events.clone();
    conn.connect_raw(Some("127.0.0.1"), port, move |c, event| {
        ev.borrow_mut().push(event.kind);
        match event.kind {
            EventKind::RawConnect => c.disconnect(),
            _ => c.context().stop(),
        }
    })
    .unwrap();
    ctx.run(); // stopped by the handler
    assert_eq!(*events.borrow(), [EventKind::RawConnect, EventKind::Disconnect]);
}

#[test]
fn stanza_and_id_handlers_receive_stanzas() {
    let port = fake_server(Some(format!(
        "{STREAM_HEADER}<message from='a@localhost' type='chat' id='m1'><body>hi</body></message>"
    )));
    let ctx = Context::new();
    let conn = raw_conn(&ctx);
    let bodies = Rc::new(RefCell::new(Vec::<String>::new()));
    let ids = Rc::new(Cell::new(0));
    let deleted_called = Rc::new(Cell::new(false));
    let disconnected = Rc::new(Cell::new(false));
    let (b, i, d, dc) = (bodies.clone(), ids.clone(), deleted_called.clone(), disconnected.clone());
    conn.connect_raw(Some("127.0.0.1"), port, move |c, event| match event.kind {
        EventKind::RawConnect => c.open_stream_default().unwrap(),
        EventKind::Connect => {
            let b = b.clone();
            c.handler_add(None, Some("message"), Some("chat"), move |c, st| {
                b.borrow_mut().push(st.child_by_name("body").unwrap().text().unwrap());
                assert_eq!(st.from(), Some("a@localhost"));
                c.disconnect();
                false
            })
            .unwrap();
            let i = i.clone();
            c.id_handler_add("m1", move |_, st| {
                assert_eq!(st.id(), Some("m1"));
                i.set(i.get() + 1);
                true
            })
            .unwrap();
            let d = d.clone();
            let gone = c.handler_add(None, Some("message"), None, move |_, _| {
                d.set(true);
                true
            })
            .unwrap();
            c.handler_delete(&gone);
        }
        _ => dc.set(true),
    })
    .unwrap();
    run_until(&ctx, || disconnected.get());
    assert_eq!(*bodies.borrow(), ["hi"]);
    assert_eq!(ids.get(), 1);
    assert!(!deleted_called.get(), "deleted handler must not fire");
}

#[test]
fn timed_handler_repeats_until_false() {
    let port = fake_server(None);
    let ctx = Context::new();
    let conn = raw_conn(&ctx);
    let ticks = Rc::new(Cell::new(0));
    let disconnected = Rc::new(Cell::new(false));
    let (t, dc) = (ticks.clone(), disconnected.clone());
    conn.connect_raw(Some("127.0.0.1"), port, move |c, event| match event.kind {
        EventKind::RawConnect => {
            let t = t.clone();
            c.timed_handler_add(Duration::from_millis(5), move |c| {
                t.set(t.get() + 1);
                if t.get() == 3 {
                    c.disconnect();
                }
                t.get() < 3
            });
        }
        _ => dc.set(true),
    })
    .unwrap();
    run_until(&ctx, || disconnected.get());
    assert_eq!(ticks.get(), 3);
}

#[test]
fn dropping_connected_connection_fires_disconnect() {
    let port = fake_server(None);
    let ctx = Context::new();
    let connected = Rc::new(Cell::new(false));
    let disconnected = Rc::new(Cell::new(false));
    {
        let conn = raw_conn(&ctx);
        let (c1, d1) = (connected.clone(), disconnected.clone());
        conn.connect_raw(Some("127.0.0.1"), port, move |_, event| match event.kind {
            EventKind::RawConnect => c1.set(true),
            _ => d1.set(true),
        })
        .unwrap();
        run_until(&ctx, || connected.get());
    } // drop -> xmpp_conn_release -> disconnect handler while closure alive
    assert!(disconnected.get());
}

#[test]
fn connection_failure_reports_event() {
    // Bind then drop to get a (very likely) closed port.
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let ctx = Context::new();
    let conn = raw_conn(&ctx);
    let got = Rc::new(Cell::new(None));
    let g = got.clone();
    conn.connect_raw(Some("127.0.0.1"), port, move |_, event| {
        g.set(Some((event.kind, event.error)));
    })
    .unwrap();
    run_until(&ctx, || got.get().is_some());
    let (kind, error) = got.get().unwrap();
    assert_eq!(kind, EventKind::Disconnect);
    assert_ne!(error, 0);
}
