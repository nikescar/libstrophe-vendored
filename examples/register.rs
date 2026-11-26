//! Port of libstrophe `examples/register.c`: in-band registration (XEP-0077).
//!
//! Uses a "raw" connection to reach the server without an account, then asks
//! the server to register a new one. After the server's instructions arrive,
//! you are prompted for each field (username, password, ...). Press enter
//! without typing to skip a field. Forms are not implemented, so it won't work
//! in complicated scenarios.
#![forbid(unsafe_code)]

mod common;
use common::XmppConfig;
use libstrophe_vendored::xmpp::{
    Conn, ConnEvent, Connection, Context, EventKind, HandlerId, LogLevel, Result, Stanza, StanzaRef,
    NS_REGISTER, NS_STREAMS, NS_TLS,
};
use std::io::Write;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

/// private data
struct Reg {
    jid: String,
}

const FEATURES_TIMEOUT: Duration = Duration::from_millis(5000);

fn iq_reg_send_form(conn: &Conn, stanza: &StanzaRef) -> Result<()> {
    let ctx = conn.context();
    let Some(query) = stanza.child_by_name("query") else {
        conn.disconnect();
        return Ok(());
    };

    let mut form = Stanza::new(ctx);
    form.set_name("query")?;
    form.set_ns(NS_REGISTER)?;
    for next in query.children() {
        let Some(name) = next.name() else { continue }; // text node
        if name == "instructions" {
            println!("instructions: {}", next.text().unwrap_or_default());
            continue;
        }
        print!("{name}: ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            continue;
        }
        let value = line.strip_suffix('\n').unwrap_or(&line);
        if !value.is_empty() {
            let mut elem = Stanza::new(ctx);
            let mut text = Stanza::new(ctx);
            text.set_text(value)?;
            elem.set_name(name)?;
            elem.add_child(text)?;
            form.add_child(elem)?;
        }
    }

    if form.children().next().is_none() {
        eprintln!("DEBUG: nothing to send");
        conn.disconnect();
    } else {
        let mut iq = Stanza::iq(ctx, "set", "reg2")?;
        iq.add_child(form)?;
        conn.send(&iq);
    }
    Ok(())
}

fn iq_reg2_cb(conn: &Conn, stanza: &StanzaRef) -> bool {
    match stanza.stanza_type() {
        None | Some("error") => eprintln!("DEBUG: error during registration"),
        Some("result") => eprintln!("DEBUG: successful registration"),
        Some(t) => eprintln!("DEBUG: expected type 'result', but got {t}"),
    }
    conn.disconnect();
    false
}

fn iq_reg_cb(conn: &Conn, stanza: &StanzaRef) -> bool {
    match stanza.stanza_type() {
        None | Some("error") => {
            eprintln!("DEBUG: error during registration");
            conn.disconnect();
            return false;
        }
        Some("result") => {}
        Some(t) => {
            eprintln!("DEBUG: expected type 'result', but got {t}");
            conn.disconnect();
            return false;
        }
    }

    let registered = stanza.child_by_name("query").and_then(|q| q.child_by_name("registered"));
    if registered.is_some() {
        eprintln!("DEBUG: already registered");
        conn.disconnect();
        return false;
    }
    let result = conn.id_handler_add("reg2", iq_reg2_cb).and_then(|_| iq_reg_send_form(conn, stanza));
    if let Err(e) = result {
        eprintln!("ERROR: {e}");
        conn.disconnect();
    }
    false
}

fn handle_error(conn: &Conn, _stanza: &StanzaRef) -> bool {
    eprintln!("DEBUG: received stream error");
    conn.disconnect();
    false
}

fn handle_proceedtls_default(conn: &Conn, stanza: &StanzaRef, error_handler: &HandlerId) -> bool {
    if stanza.name() == Some("proceed") {
        eprintln!("DEBUG: proceeding with TLS");
        if conn.tls_start().is_ok() {
            conn.handler_delete(error_handler);
            let _ = conn.open_stream_default();
        } else {
            eprintln!("DEBUG: TLS failed");
            // failed tls spoils the connection, so disconnect
            conn.disconnect();
        }
    }
    false
}

fn handle_missing_features(conn: &Conn) -> bool {
    eprintln!("DEBUG: timeout");
    conn.disconnect();
    false
}

fn handle_features(conn: &Conn, stanza: &StanzaRef, reg: &Reg, timeout: &HandlerId, error: &HandlerId) -> Result<()> {
    let ctx = conn.context();
    conn.handler_delete(timeout);

    // secure connection if possible
    if stanza.child_by_name("starttls").is_some_and(|c| c.ns() == Some(NS_TLS)) {
        eprintln!("DEBUG: server supports TLS, try to establish");
        let mut child = Stanza::new(ctx);
        child.set_name("starttls")?;
        child.set_ns(NS_TLS)?;
        let error = error.clone();
        conn.handler_add(Some(NS_TLS), None, None, move |c, s| handle_proceedtls_default(c, s, &error))?;
        conn.send(&child);
        return Ok(());
    }

    // check whether server supports in-band registration
    // (condition kept identical to register.c)
    if stanza.child_by_name("register").is_some_and(|c| c.ns() == Some(NS_REGISTER)) {
        eprintln!("DEBUG: server doesn't support in-band registration");
        conn.disconnect();
        return Ok(());
    }

    eprintln!("DEBUG: server supports in-band registration");
    let domain = ctx.jid_domain(&reg.jid).unwrap_or_default();
    let mut iq = Stanza::iq(ctx, "get", "reg1")?;
    iq.set_to(&domain)?;
    let mut child = Stanza::new(ctx);
    child.set_name("query")?;
    child.set_ns(NS_REGISTER)?;
    iq.add_child(child)?;

    conn.handler_add(Some(NS_REGISTER), Some("iq"), None, iq_reg_cb)?;
    conn.send(&iq);
    Ok(())
}

fn on_stream_opened(conn: &Conn, reg: &Rc<Reg>) -> Result<()> {
    let secured = conn.is_secured();
    eprintln!("DEBUG: connection is {}.", if secured { "secured" } else { "NOT secured" });

    // setup handler for stream:error
    let error = conn.handler_add(Some(NS_STREAMS), Some("error"), None, handle_error)?;

    // setup handlers for incoming <stream:features>
    let timeout = conn.timed_handler_add(FEATURES_TIMEOUT, handle_missing_features);
    let reg = reg.clone();
    conn.handler_add(Some(NS_STREAMS), Some("features"), None, move |c, s| {
        if let Err(e) = handle_features(c, s, &reg, &timeout, &error) {
            eprintln!("ERROR: {e}");
            c.disconnect();
        }
        false
    })?;
    Ok(())
}

fn conn_handler(conn: &Conn, event: &ConnEvent<'_>, reg: &Rc<Reg>) {
    match event.kind {
        EventKind::RawConnect => {
            eprintln!("DEBUG: raw connection established");
            let _ = conn.open_stream_default();
        }
        EventKind::Connect => {
            eprintln!("DEBUG: stream opened");
            if let Err(e) = on_stream_opened(conn, reg) {
                eprintln!("ERROR: {e}");
                conn.disconnect();
            }
        }
        _ => {
            eprintln!("DEBUG: disconnected");
            conn.context().stop();
        }
    }
}

fn main() -> std::result::Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Parse CLI args
    let (cli_jid, cli_host) = match args.len() {
        2 => (Some(args[1].as_str()), None),
        3 => (Some(args[1].as_str()), Some(args[2].as_str())),
        1 => (None, None),
        _ => {
            eprintln!(
                "Usage: {} [<jid> [<host>]]\n\n\
                 Environment variables (priority over CLI):\n  \
                 XMPP_JID    Jabber ID or domain to register\n  \
                 XMPP_HOST   Server hostname\n",
                args.first().map_or("register", String::as_str)
            );
            return Ok(ExitCode::FAILURE);
        }
    };

    // Get configuration (ENV > CLI)
    let config = XmppConfig::from_env_and_args(cli_jid, None, cli_host, None, None);
    if config.jid.is_empty() {
        eprintln!("Error: JID or domain required");
        return Ok(ExitCode::FAILURE);
    }

    let jid = &config.jid;
    let host = config.host.as_deref();

    let ctx = Context::with_default_logger(LogLevel::Debug);
    let conn = Connection::new(&ctx);

    // jid can be a jid or domain for "raw" connection
    let domain = ctx.jid_domain(jid).ok_or("invalid jid")?;
    conn.set_jid(&domain)?;

    // private data
    let reg = Rc::new(Reg { jid: jid.clone() });

    conn.connect_raw(host, 0, move |c, e| conn_handler(c, e, &reg))?;
    ctx.run();
    Ok(ExitCode::SUCCESS)
}
