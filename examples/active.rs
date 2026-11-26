//! Port of libstrophe `examples/active.c`: list active sessions (disco#items).
#![forbid(unsafe_code)]

mod common;
use common::XmppConfig;
use libstrophe_vendored::xmpp::{
    Conn, ConnEvent, Connection, Context, EventKind, Result, Stanza, StanzaRef, NS_DISCO_ITEMS,
};
use std::process::ExitCode;

fn handle_reply(conn: &Conn, stanza: &StanzaRef) -> bool {
    if stanza.stanza_type() == Some("error") {
        eprintln!("ERROR: query failed");
    } else {
        println!("Active Sessions:");
        if let Some(query) = stanza.child_by_name("query") {
            for item in query.children() {
                println!("\t {}", item.attribute("jid").unwrap_or("(null)"));
            }
        }
        println!("END OF LIST");
    }
    // disconnect
    conn.disconnect();
    false
}

fn send_request(conn: &Conn) -> Result<()> {
    let ctx = conn.context();
    // create iq stanza for request
    let mut iq = Stanza::iq(ctx, "get", "active1")?;
    iq.set_to("xxxxxxxxx.com")?;
    let mut query = Stanza::new(ctx);
    query.set_name("query")?;
    query.set_ns(NS_DISCO_ITEMS)?;
    query.set_attribute("node", "sessions")?;
    // query now belongs to iq
    iq.add_child(query)?;
    // set up reply handler
    conn.id_handler_add("active1", handle_reply)?;
    // send out the stanza; it is released on drop
    conn.send(&iq);
    Ok(())
}

fn conn_handler(conn: &Conn, event: &ConnEvent<'_>) {
    if event.kind == EventKind::Connect {
        eprintln!("DEBUG: connected");
        if let Err(e) = send_request(conn) {
            eprintln!("ERROR: {e}");
            conn.disconnect();
        }
    } else {
        eprintln!("DEBUG: disconnected");
        conn.context().stop();
    }
}

fn main() -> std::result::Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Parse CLI args
    let (cli_jid, cli_pass) = match args.len() {
        3 => (Some(args[1].as_str()), Some(args[2].as_str())),
        1 => (None, None),
        _ => {
            eprint!(
                "Usage: active [<jid> <pass>]\n\n\
                 Environment variables (priority over CLI):\n  \
                 XMPP_JID    Jabber ID\n  \
                 XMPP_PASS   Password\n\n"
            );
            return Ok(ExitCode::FAILURE);
        }
    };

    // Get configuration (ENV > CLI)
    let config = XmppConfig::from_env_and_args(cli_jid, cli_pass, None, None, None);
    if !config.is_valid() {
        eprint!("Error: JID and password required\n");
        return Ok(ExitCode::FAILURE);
    }

    let ctx = Context::new();
    let conn = Connection::new(&ctx);
    // also you can disable TLS support or force legacy SSL
    // connection without STARTTLS: see Conn::set_flags() or examples/basic.rs

    // setup authentication information
    conn.set_jid(&config.jid)?;
    conn.set_pass(&config.password)?;

    // initiate connection
    conn.connect_client(None, 0, conn_handler)?;
    // start the event loop
    ctx.run();
    Ok(ExitCode::SUCCESS)
}
