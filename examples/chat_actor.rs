//! Actor-thread chat client: the pattern for embedding libstrophe in an app
//! (e.g. a mobile view-model) where other threads want to send messages.
//!
//! `Context` is `!Send`, so one thread owns the context, the connection and
//! the event loop. Callbacks build stanzas from `conn.context()`:
//! - initial `<presence/>` on connect (without it the server treats this
//!   resource as unavailable: contacts see it offline and messages to the
//!   bare JID go to offline storage instead of here),
//! - an echo auto-reply via `stanza.reply()`,
//! - messages and presence updates queued from other threads over a channel
//!   and drained by a timed handler.
//!
//! stdin commands; EOF (Ctrl-D) disconnects, and the server then broadcasts
//! unavailable presence for us:
//!   <jid> <message>                      send a chat message
//!   /presence [away|chat|dnd|xa] [text]  broadcast availability/status
#![forbid(unsafe_code)]

use libstrophe_vendored::xmpp::{
    Conn, Connection, Context, ContextRef, EventKind, LogLevel, Result, Stanza, StanzaRef,
};
use std::io::BufRead;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

/// How often the actor checks the outgoing queue.
const QUEUE_POLL: Duration = Duration::from_millis(100);

/// What other threads ask the actor to send.
enum Outgoing {
    Message { to: String, body: String },
    Presence { show: Option<String>, status: Option<String> },
}

fn text_element<'a>(ctx: &'a ContextRef, name: &str, text: &str) -> Result<Stanza<'a>> {
    let mut elem = Stanza::new(ctx);
    elem.set_name(name)?;
    let mut t = Stanza::new(ctx);
    t.set_text(text)?;
    elem.add_child(t)?;
    Ok(elem)
}

/// Presence without `to` is broadcast by the server to all subscribed contacts.
fn send_presence(conn: &Conn, show: Option<&str>, status: Option<&str>) -> Result<()> {
    let ctx = conn.context();
    let mut pres = Stanza::presence(ctx)?;
    if let Some(show) = show {
        pres.add_child(text_element(ctx, "show", show)?)?;
    }
    if let Some(status) = status {
        pres.add_child(text_element(ctx, "status", status)?)?;
    }
    conn.send(&pres);
    Ok(())
}

fn auto_reply(conn: &Conn, stanza: &StanzaRef) -> Result<()> {
    let Some(body) = stanza.body() else { return Ok(()) };
    println!("{}: {body}", stanza.from().unwrap_or("(unknown)"));
    let mut reply = stanza.reply()?; // `to` = original `from`, same id and type
    reply.set_body(&format!("echo: {body}"))?;
    conn.send(&reply);
    Ok(())
}

fn send_outgoing(conn: &Conn, out: Outgoing) -> Result<()> {
    match out {
        Outgoing::Message { to, body } => {
            let mut msg = Stanza::message(conn.context(), Some("chat"), &to, None)?;
            msg.set_body(&body)?;
            conn.send(&msg);
            println!("-> {to}: {body}");
        }
        Outgoing::Presence { show, status } => {
            send_presence(conn, show.as_deref(), status.as_deref())?;
            println!("-> presence {} {}", show.as_deref().unwrap_or("available"), status.unwrap_or_default());
        }
    }
    Ok(())
}

/// Send everything queued so far. Returns `false` once all senders are gone.
fn drain_outgoing(conn: &Conn, rx: &Receiver<Outgoing>) -> Result<bool> {
    loop {
        match rx.try_recv() {
            Ok(out) => send_outgoing(conn, out)?,
            Err(TryRecvError::Empty) => return Ok(true),
            Err(TryRecvError::Disconnected) => return Ok(false),
        }
    }
}

fn on_connect(conn: &Conn, outgoing: Receiver<Outgoing>) -> Result<()> {
    conn.handler_add(None, Some("message"), Some("chat"), |c, st| {
        if let Err(e) = auto_reply(c, st) {
            eprintln!("ERROR: auto-reply: {e}");
        }
        true
    })?;
    conn.timed_handler_add(QUEUE_POLL, move |c| match drain_outgoing(c, &outgoing) {
        Ok(true) => true,
        Ok(false) => {
            c.disconnect();
            false
        }
        Err(e) => {
            eprintln!("ERROR: send: {e}");
            true
        }
    });
    send_presence(conn, None, None)
}

/// Owns everything libstrophe; runs until disconnected.
fn actor(jid: String, pass: String, host: Option<String>, port: u16, outgoing: Receiver<Outgoing>) -> Result<()> {
    let ctx = Context::with_default_logger(LogLevel::Info);
    let conn = Connection::new(&ctx);
    conn.set_jid(&jid)?;
    conn.set_pass(&pass)?;
    let mut outgoing = Some(outgoing);
    conn.connect_client(host.as_deref(), port, move |c, event| match event.kind {
        EventKind::Connect => {
            println!("connected as {}", c.bound_jid().unwrap_or("?"));
            // Connect fires once per connection; the queue moves into the timed handler.
            if let Some(rx) = outgoing.take() {
                if let Err(e) = on_connect(c, rx) {
                    eprintln!("ERROR: {e}");
                    c.disconnect();
                }
            }
        }
        _ => {
            println!("disconnected");
            c.context().stop();
        }
    })?;
    ctx.run();
    Ok(())
}

/// Parse one stdin line; `None` if malformed.
fn parse_command(line: &str) -> Option<Outgoing> {
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    if let Some(rest) = line.strip_prefix("/presence") {
        let rest = rest.trim();
        let (first, tail) = rest.split_once(' ').unwrap_or((rest, ""));
        return Some(match first {
            "away" | "chat" | "dnd" | "xa" => {
                Outgoing::Presence { show: Some(first.to_owned()), status: non_empty(tail.trim()) }
            }
            _ => Outgoing::Presence { show: None, status: non_empty(rest) },
        });
    }
    let (to, body) = line.split_once(' ')?;
    (!to.is_empty()).then(|| Outgoing::Message { to: to.to_owned(), body: body.to_owned() })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(jid), Some(pass)) = (args.next(), args.next()) else {
        eprintln!("Usage: chat_actor <jid> <pass> [<host> [<port>]]");
        std::process::exit(1);
    };
    let host = args.next();
    let port = args.next().and_then(|p| p.parse().ok()).unwrap_or(0);

    let (tx, rx) = mpsc::channel();
    let actor = std::thread::spawn(move || actor(jid, pass, host, port, rx));

    // Any thread can hold a Sender; here the main thread feeds it from stdin.
    for line in std::io::stdin().lock().lines().map_while(std::io::Result::ok) {
        match parse_command(&line) {
            Some(out) => {
                if tx.send(out).is_err() {
                    break; // actor already gone
                }
            }
            None => eprintln!("format: <jid> <message> | /presence [away|chat|dnd|xa] [status]"),
        }
    }
    drop(tx); // EOF: actor disconnects on its next queue poll

    match actor.join() {
        Ok(Err(e)) => eprintln!("ERROR: {e}"),
        Err(_) => eprintln!("ERROR: actor thread panicked"),
        Ok(Ok(())) => {}
    }
}

#[test]
fn parse_commands() {
    let p = |l| match parse_command(l) {
        Some(Outgoing::Presence { show, status }) => format!("P {show:?} {status:?}"),
        Some(Outgoing::Message { to, body }) => format!("M {to} {body}"),
        None => "none".into(),
    };
    assert_eq!(p("/presence"), "P None None");
    assert_eq!(p("/presence away In a meeting"), r#"P Some("away") Some("In a meeting")"#);
    assert_eq!(p("/presence dnd"), r#"P Some("dnd") None"#);
    assert_eq!(p("/presence back soon"), r#"P None Some("back soon")"#);
    assert_eq!(p("bob@x hi there"), "M bob@x hi there");
    assert_eq!(p("nospace"), "none");
    assert_eq!(p(" leading"), "none");
}
