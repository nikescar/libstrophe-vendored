//! Port of libstrophe `examples/component.c`: connect as an external component.
#![forbid(unsafe_code)]

use libstrophe_vendored::xmpp::{Conn, ConnEvent, Connection, Context, EventKind, LogLevel};
use std::process::ExitCode;

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

fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if !(4..=5).contains(&args.len()) {
        eprint!("Usage: component <jid> <pass> <host> [<port>]\n\n");
        return Ok(ExitCode::FAILURE);
    }
    let (jid, pass, host) = (&args[1], &args[2], &args[3]);
    let mut port = 0u16;
    if let Some(p) = args.get(4) {
        match p.parse::<i16>() {
            Ok(tmp) if tmp >= 0 => port = tmp as u16,
            _ => {
                eprintln!("Invalid value of <port> [{p}].");
                return Ok(ExitCode::FAILURE);
            }
        }
    }

    // pass Context::new() instead to silence output
    let ctx = Context::with_default_logger(LogLevel::Debug);
    let conn = Connection::new(&ctx);

    // setup authentication information
    conn.set_jid(jid)?;
    conn.set_pass(pass)?;

    // initiate connection
    conn.connect_component(host, port, conn_handler)?;
    // enter the event loop - our connect handler will trigger an exit
    ctx.run();
    Ok(ExitCode::SUCCESS)
}
