//! Port of libstrophe `examples/basic.c`: connect, then disconnect.
#![forbid(unsafe_code)]

use libstrophe_vendored::xmpp::{Conn, ConnEvent, ConnFlags, Connection, Context, EventKind, LogLevel};
use std::process::exit;

/// define a handler for connection events
fn conn_handler(conn: &Conn, event: &ConnEvent<'_>) {
    if event.kind == EventKind::Connect {
        eprintln!("DEBUG: connected");
        conn.disconnect();
    } else {
        eprintln!("DEBUG: disconnected");
        conn.context().stop();
    }
}

fn usage(exit_code: i32) -> ! {
    eprint!(
        "Usage: basic [options] [<host> [<port>]]\n\n\
         Options:\n  \
         --jid <jid>              The JID to use to authenticate.\n  \
         --pass <pass>            The password of the JID.\n  \
         --disable-tls            Disable TLS.\n  \
         --mandatory-tls          Deny plaintext connection.\n  \
         --trust-tls              Trust TLS certificate.\n  \
         --legacy-ssl             Use old style SSL.\n  \
         --legacy-auth            Allow legacy authentication.\n\
         Note: --disable-tls conflicts with --mandatory-tls or --legacy-ssl\n"
    );
    exit(exit_code)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (mut jid, mut password) = (None, None);
    let mut flags = ConnFlags::empty();

    // take a jid and password on the command line
    let mut i = 1;
    while i < args.len() {
        let has_value = i + 1 < args.len();
        match args[i].as_str() {
            "--help" => usage(0),
            "--disable-tls" => flags |= ConnFlags::DISABLE_TLS,
            "--mandatory-tls" => flags |= ConnFlags::MANDATORY_TLS,
            "--trust-tls" => flags |= ConnFlags::TRUST_TLS,
            "--legacy-ssl" => flags |= ConnFlags::LEGACY_SSL,
            "--legacy-auth" => flags |= ConnFlags::LEGACY_AUTH,
            "--jid" if has_value => {
                i += 1;
                jid = Some(args[i].clone());
            }
            "--pass" if has_value => {
                i += 1;
                password = Some(args[i].clone());
            }
            _ => break,
        }
        i += 1;
    }
    if jid.is_none() || args.len() - i > 2 {
        usage(1);
    }
    let host = args.get(i).map(String::as_str);
    let port = args.get(i + 1).map_or(0, |p| p.parse::<u16>().unwrap_or(0));

    // pass Context::new() instead to silence output
    let ctx = Context::with_default_logger(LogLevel::Debug);
    let conn = Connection::new(&ctx);

    // configure connection properties (optional)
    conn.set_flags(flags)?;
    // setup authentication information
    if let Some(jid) = &jid {
        conn.set_jid(jid)?;
    }
    if let Some(password) = &password {
        conn.set_pass(password)?;
    }

    // initiate connection
    if conn.connect_client(host, port, conn_handler).is_ok() {
        // enter the event loop - our connect handler will trigger an exit
        ctx.run();
    }
    // connection and context are released on drop
    Ok(())
}
