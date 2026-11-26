//! Port of libstrophe `examples/basic.c`: connect, then disconnect.
#![forbid(unsafe_code)]

mod common;
use common::XmppConfig;
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
         --legacy-auth            Allow legacy authentication.\n\n\
         Environment variables (priority over CLI):\n  \
         XMPP_JID                 Jabber ID\n  \
         XMPP_PASS                Password\n  \
         XMPP_HOST                Server hostname\n  \
         XMPP_PORT                Server port\n\n\
         Note: --disable-tls conflicts with --mandatory-tls or --legacy-ssl\n"
    );
    exit(exit_code)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (mut cli_jid, mut cli_pass, mut cli_host, mut cli_port) = (None, None, None, None);
    let mut flags = ConnFlags::empty();

    // Parse command-line arguments
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
                cli_jid = Some(args[i].as_str());
            }
            "--pass" if has_value => {
                i += 1;
                cli_pass = Some(args[i].as_str());
            }
            _ => break,
        }
        i += 1;
    }

    // Parse positional host/port arguments
    if i < args.len() {
        cli_host = Some(args[i].as_str());
        if i + 1 < args.len() {
            cli_port = args[i + 1].parse::<u16>().ok();
        }
    }

    // Get configuration (ENV > CLI)
    let config = XmppConfig::from_env_and_args(cli_jid, cli_pass, cli_host, cli_port, None);
    if !config.is_valid() {
        eprintln!("Error: JID and password required\n");
        usage(1);
    }

    // pass Context::new() instead to silence output
    let ctx = Context::with_default_logger(LogLevel::Debug);
    let conn = Connection::new(&ctx);

    // configure connection properties
    conn.set_flags(flags)?;
    conn.set_jid(&config.jid)?;
    conn.set_pass(&config.password)?;

    // initiate connection
    let host = config.host.as_deref();
    let port = config.port.unwrap_or(0);
    if conn.connect_client(host, port, conn_handler).is_ok() {
        // enter the event loop - our connect handler will trigger an exit
        ctx.run();
    }
    // connection and context are released on drop
    Ok(())
}
