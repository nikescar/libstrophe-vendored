//! Port of libstrophe `examples/bot.c`: echo bot with jabber:iq:version and
//! Stream Management based reconnect.
#![forbid(unsafe_code)]

mod common;
use common::XmppConfig;
use libstrophe_vendored::xmpp::{
    Conn, ConnEvent, ConnFlags, Connection, Context, ContextRef, EventKind, LogLevel, Result, Stanza,
    StanzaRef,
};
use std::cell::Cell;
use std::io::Write;
use std::process::exit;
use std::rc::Rc;
use std::time::Duration;

fn text_element<'a>(ctx: &'a ContextRef, name: &str, text: &str) -> Result<Stanza<'a>> {
    let mut elem = Stanza::new(ctx);
    elem.set_name(name)?;
    let mut t = Stanza::new(ctx);
    t.set_text(text)?;
    elem.add_child(t)?;
    Ok(elem)
}

fn send_version(conn: &Conn, stanza: &StanzaRef) -> Result<()> {
    let ctx = conn.context();
    let mut reply = stanza.reply()?;
    reply.set_type("result")?;

    let mut query = Stanza::new(ctx);
    query.set_name("query")?;
    if let Some(ns) = stanza.children().next().and_then(|c| c.ns()) {
        query.set_ns(ns)?;
    }
    query.add_child(text_element(ctx, "name", "libstrophe example bot")?)?;
    query.add_child(text_element(ctx, "version", "1.0")?)?;
    reply.add_child(query)?;

    conn.send(&reply);
    Ok(())
}

fn version_handler(conn: &Conn, stanza: &StanzaRef) -> bool {
    println!("Received version request from {}", stanza.from().unwrap_or("(null)"));
    if let Err(e) = send_version(conn, stanza) {
        eprintln!("ERROR: {e}");
    }
    true
}

fn quit_handler(conn: &Conn) -> bool {
    conn.disconnect();
    false
}

fn send_echo(conn: &Conn, stanza: &StanzaRef, intext: &str, reconnect: &Cell<bool>) -> Result<()> {
    let mut reply = stanza.reply()?;
    if reply.stanza_type().is_none() {
        reply.set_type("chat")?;
    }

    let replytext = match intext {
        "quit" => {
            conn.timed_handler_add(Duration::from_millis(500), quit_handler);
            "bye!".to_owned()
        }
        "reconnect" => {
            reconnect.set(true);
            conn.timed_handler_add(Duration::from_millis(500), quit_handler);
            "alright, let's see what happens!".to_owned()
        }
        _ => format!("{intext} to you too!"),
    };

    reply.set_body(&replytext)?;
    conn.send(&reply);
    Ok(())
}

fn message_handler(conn: &Conn, stanza: &StanzaRef, reconnect: &Cell<bool>) -> bool {
    let Some(body) = stanza.child_by_name("body") else { return true };
    if stanza.stanza_type() == Some("error") {
        return true;
    }

    let intext = body.text().unwrap_or_default();
    println!("Incoming message from {}: {}", stanza.from().unwrap_or("(null)"), intext);

    if let Err(e) = send_echo(conn, stanza, &intext, reconnect) {
        eprintln!("ERROR: {e}");
    }
    true
}

fn on_connect(conn: &Conn, reconnect: &Rc<Cell<bool>>) -> Result<()> {
    conn.handler_add(Some("jabber:iq:version"), Some("iq"), None, version_handler)?;
    let reconnect = reconnect.clone();
    conn.handler_add(None, Some("message"), None, move |c, s| message_handler(c, s, &reconnect))?;

    // Send initial <presence/> so that we appear online to contacts
    let pres = Stanza::presence(conn.context())?;
    conn.send(&pres);
    Ok(())
}

/// define a handler for connection events
fn conn_handler(conn: &Conn, event: &ConnEvent<'_>, reconnect: &Rc<Cell<bool>>) {
    if event.kind == EventKind::Connect {
        eprintln!("DEBUG: connected");
        if let Err(e) = on_connect(conn, reconnect) {
            eprintln!("ERROR: {e}");
        }
    } else {
        eprintln!("DEBUG: disconnected");
        conn.context().stop();
    }
}

/// Reads the key password from stdin. Unlike C getpass(), input is echoed.
fn password_callback(conn: &Conn, pw_max: usize) -> Option<String> {
    println!("Trying to unlock {}", conn.keyfile().unwrap_or("(null)"));
    eprint!("Please enter password: ");
    std::io::stderr().flush().ok();
    let mut pass = String::new();
    std::io::stdin().read_line(&mut pass).ok()?;
    let pass = pass.trim_end_matches(['\r', '\n']).to_owned();
    (pass.len() < pw_max).then_some(pass)
}

fn usage(exit_code: i32) -> ! {
    eprint!(
        "Usage: bot [options] [<host> [<port>]]\n\n\
         Options:\n  \
         --jid <jid>              The JID to use to authenticate.\n  \
         --pass <pass>            The password of the JID.\n  \
         --tls-cert <cert>        Path to client certificate.\n  \
         --tls-key <key>          Path to private key or P12 file.\n  \
         --tcp-keepalive          Configure TCP keepalive.\n  \
         --disable-tls            Disable TLS.\n  \
         --mandatory-tls          Deny plaintext connection.\n  \
         --trust-tls              Trust TLS certificate.\n  \
         --legacy-ssl             Use old style SSL.\n  \
         --legacy-auth            Allow legacy authentication.\n  \
         --zlib                   Enable compression via zlib.\n  \
         --dont-reset             When using zlib, don't do a full-flush after compression.\n\n\
         Environment variables (priority over CLI):\n  \
         XMPP_JID                 Jabber ID\n  \
         XMPP_PASS                Password\n  \
         XMPP_HOST                Server hostname\n  \
         XMPP_PORT                Server port\n\n\
         Note: --disable-tls conflicts with --mandatory-tls or --legacy-ssl\n"
    );
    exit(exit_code)
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (mut cli_jid, mut cli_pass, mut cli_host, mut cli_port) = (None, None, None, None);
    let (mut cert, mut key) = (None, None);
    let mut flags = ConnFlags::empty();
    let mut tcp_keepalive = false;

    // Parse command-line arguments
    let mut i = 1;
    while i < args.len() {
        let has_value = i + 1 < args.len();
        let value = |i: &mut usize| {
            *i += 1;
            Some(args[*i].clone())
        };
        match args[i].as_str() {
            "--help" => usage(0),
            "--disable-tls" => flags |= ConnFlags::DISABLE_TLS,
            "--mandatory-tls" => flags |= ConnFlags::MANDATORY_TLS,
            "--trust-tls" => flags |= ConnFlags::TRUST_TLS,
            "--legacy-ssl" => flags |= ConnFlags::LEGACY_SSL,
            "--legacy-auth" => flags |= ConnFlags::LEGACY_AUTH,
            "--zlib" => flags |= ConnFlags::ENABLE_COMPRESSION,
            "--dont-reset" => flags |= ConnFlags::COMPRESSION_DONT_RESET,
            "--jid" if has_value => {
                i += 1;
                cli_jid = Some(args[i].as_str());
            }
            "--pass" if has_value => {
                i += 1;
                cli_pass = Some(args[i].as_str());
            }
            "--tls-cert" if has_value => cert = value(&mut i),
            "--tls-key" if has_value => key = value(&mut i),
            "--tcp-keepalive" => tcp_keepalive = true,
            _ => break,
        }
        i += 1;
    }

    // Parse positional host/port
    if i < args.len() {
        cli_host = Some(args[i].as_str());
        if i + 1 < args.len() {
            cli_port = args[i + 1].parse::<u16>().ok();
        }
    }

    // Get configuration (ENV > CLI)
    let config = XmppConfig::from_env_and_args(cli_jid, cli_pass, cli_host, cli_port, None);
    if !config.is_valid() && key.is_none() {
        eprintln!("Error: JID and password required (or client certificate)\n");
        usage(1);
    }

    let host = config.host.as_deref();
    let port = config.port.unwrap_or(0);

    // pass Context::new() instead to silence output
    let ctx = Context::with_default_logger(LogLevel::Debug);
    let reconnect = Rc::new(Cell::new(false));
    let mut sm_state = None;

    loop {
        reconnect.set(false);
        let conn = Connection::new(&ctx);

        // configure connection properties (optional)
        conn.set_flags(flags)?;

        // ask for a password if key is protected
        conn.set_password_callback(password_callback);
        // try at max 3 times in case the user enters the password wrong
        conn.set_password_retries(3);
        // setup authentication information
        if let Some(key) = &key {
            conn.set_client_cert(cert.as_deref(), key)?;
        }
        if config.is_valid() {
            conn.set_jid(&config.jid)?;
            conn.set_pass(&config.password)?;
        }

        // enable TCP keepalive, using canned callback function
        if tcp_keepalive {
            conn.set_sockopt_keepalive();
        }

        // set Stream-Management state if available
        if let Some(state) = sm_state.take() {
            if let Err(e) = conn.set_sm_state(state) {
                eprintln!("ERROR: can't restore SM state: {e}");
            }
        }

        // initiate connection
        let r = reconnect.clone();
        if conn.connect_client(host, port, move |c, e| conn_handler(c, e, &r)).is_ok() {
            // enter the event loop - our connect handler will trigger an exit
            ctx.run();
        }

        // save the Stream-Management state if we should re-connect
        if reconnect.get() {
            sm_state = conn.take_sm_state();
        }

        // release our connection
        drop(conn);

        if !reconnect.get() {
            break;
        }
    }
    Ok(())
}
