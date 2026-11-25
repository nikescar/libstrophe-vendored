//! Port of libstrophe `examples/complex.c`: connect with TLS options, print
//! the server certificate, optional certfail handler and TCP keepalive.
#![forbid(unsafe_code)]

use libstrophe_vendored::xmpp::{
    Conn, ConnEvent, ConnFlags, Connection, Context, EventKind, LogLevel, Socket, TlsCertRef, CERT_ELEMENTS,
};
use std::io::Write;
use std::process::exit;
use std::time::Duration;

// hardcoded TCP keepalive timeout and interval
const KA_TIMEOUT: u64 = 60;
const KA_INTERVAL: u64 = 30;
#[allow(dead_code)] // not settable on every platform
const KA_COUNT: u32 = 3;
#[allow(dead_code)]
const USER_TIMEOUT: u64 = 150;

fn print_tlscert(cert: &TlsCertRef) {
    for element in CERT_ELEMENTS {
        println!(
            "\t{:>32}: {}",
            TlsCertRef::description(element).unwrap_or(""),
            cert.get(element).unwrap_or("(null)")
        );
    }
    for name in cert.dnsnames() {
        println!("\t{:>32}: {}", "dnsName", name);
    }
    println!("PEM:\n{}", cert.pem().unwrap_or("(null)"));
}

/// define a handler for connection events
fn conn_handler(conn: &Conn, event: &ConnEvent<'_>) {
    if event.kind == EventKind::Connect {
        eprintln!("DEBUG: connected");
        let secured = conn.is_secured();
        eprintln!("DEBUG: connection is {}.", if secured { "secured" } else { "NOT secured" });
        if secured {
            match conn.peer_cert() {
                Some(cert) => print_tlscert(&cert),
                None => eprintln!("DEBUG: peer certificate not available"),
            }
        }
        conn.disconnect();
    } else {
        eprintln!("DEBUG: disconnected");
        conn.context().stop();
    }
}

fn read_line() -> Option<String> {
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line.trim_end_matches(['\r', '\n']).to_owned())
}

fn certfail_handler(cert: &TlsCertRef, errormsg: &str) -> bool {
    println!("Received certificate can't be validated!");
    println!("Reason: {errormsg}");
    print_tlscert(cert);
    print!("Do you agree to connect?\n[y(es)|n(o)]: ");
    std::io::stdout().flush().ok();
    let Some(answer) = read_line() else {
        println!("reading stdin failed");
        return false;
    };
    println!();
    answer.starts_with(['y', 'Y'])
}

/// Reads the key password from stdin. Unlike C getpass(), input is echoed.
fn password_callback(conn: &Conn, pw_max: usize) -> Option<String> {
    println!("Trying to unlock {}", conn.keyfile().unwrap_or("(null)"));
    eprint!("Please enter password: ");
    std::io::stderr().flush().ok();
    let pass = read_line()?;
    (pass.len() < pw_max).then_some(pass)
}

fn sockopt_cb(_conn: &Conn, socket: Socket<'_>) -> std::io::Result<()> {
    let sock = socket2::SockRef::from(&socket);
    eprintln!("DEBUG: setting socket options");
    // SO_KEEPALIVE + TCP_KEEPIDLE/TCP_KEEPINTVL (+ TCP_KEEPCNT where supported)
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(KA_TIMEOUT))
        .with_interval(Duration::from_secs(KA_INTERVAL));
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "freebsd"))]
    let keepalive = keepalive.with_retries(KA_COUNT);
    sock.set_tcp_keepalive(&keepalive)?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    sock.set_tcp_user_timeout(Some(Duration::from_millis(USER_TIMEOUT)))?;
    Ok(())
}

fn usage(exit_code: i32) -> ! {
    eprint!(
        "Usage: complex [options] [<host> [<port>]]\n\n\
         Options:\n  \
         --jid <jid>              The JID to use to authenticate.\n  \
         --pass <pass>            The password of the JID.\n  \
         --tls-cert <cert>        Path to client certificate.\n  \
         --tls-key <key>          Path to private key or P12 file.\n\n  \
         --capath <path>          Path to an additional CA trust store (directory).\n  \
         --cafile <path>          Path to an additional CA trust store (single file).\n  \
         --disable-tls            Disable TLS.\n  \
         --mandatory-tls          Deny plaintext connection.\n  \
         --trust-tls              Trust TLS certificate.\n  \
         --enable-certfail        Enable certfail handler.\n  \
         --legacy-ssl             Use old style SSL.\n  \
         --legacy-auth            Allow legacy authentication.\n  \
         --zlib                   Enable compression via zlib.\n  \
         --dont-reset             When using zlib, don't do a full-flush after compression.\n  \
         --verbose                Increase the verbosity level.\n  \
         --tcp-keepalive          Configure TCP keepalive.\n\n\
         Note: --disable-tls conflicts with --mandatory-tls or --legacy-ssl\n"
    );
    exit(exit_code)
}

#[derive(Default)]
struct Options {
    jid: Option<String>,
    password: Option<String>,
    cert: Option<String>,
    key: Option<String>,
    capath: Option<String>,
    cafile: Option<String>,
    flags: ConnFlags,
    tcp_keepalive: bool,
    verbosity: i32,
    certfail: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut o = Options::default();

    // take a jid and password on the command line
    let mut i = 1;
    while i < args.len() {
        let has_value = i + 1 < args.len();
        let value = |i: &mut usize| {
            *i += 1;
            Some(args[*i].clone())
        };
        match args[i].as_str() {
            "--help" => usage(0),
            "--disable-tls" => o.flags |= ConnFlags::DISABLE_TLS,
            "--mandatory-tls" => o.flags |= ConnFlags::MANDATORY_TLS,
            "--trust-tls" => o.flags |= ConnFlags::TRUST_TLS,
            "--legacy-ssl" => o.flags |= ConnFlags::LEGACY_SSL,
            "--legacy-auth" => o.flags |= ConnFlags::LEGACY_AUTH,
            "--zlib" => o.flags |= ConnFlags::ENABLE_COMPRESSION,
            "--dont-reset" => o.flags |= ConnFlags::COMPRESSION_DONT_RESET,
            "--verbose" => o.verbosity += 1,
            "--tcp-keepalive" => o.tcp_keepalive = true,
            "--enable-certfail" => o.certfail = true,
            "--jid" if has_value => o.jid = value(&mut i),
            "--pass" if has_value => o.password = value(&mut i),
            "--tls-cert" if has_value => o.cert = value(&mut i),
            "--tls-key" if has_value => o.key = value(&mut i),
            "--capath" if has_value => o.capath = value(&mut i),
            "--cafile" if has_value => o.cafile = value(&mut i),
            _ => break,
        }
        i += 1;
    }
    if (o.jid.is_none() && o.key.is_none()) || args.len() - i > 2 {
        usage(1);
    }
    let host = args.get(i).map(String::as_str);
    let port = args.get(i + 1).map_or(0, |p| p.parse::<u16>().unwrap_or(0));

    // pass Context::new() instead to silence output
    let ctx = Context::with_default_logger(LogLevel::Debug);
    ctx.set_verbosity(o.verbosity);

    let conn = Connection::new(&ctx);

    // configure connection properties (optional)
    conn.set_flags(o.flags)?;
    // configure TCP keepalive (optional)
    if o.tcp_keepalive {
        conn.set_sockopt_callback(sockopt_cb);
    }

    // ask for a password if key is protected
    conn.set_password_callback(password_callback);
    // try at max 3 times in case the user enters the password wrong
    conn.set_password_retries(3);
    // setup authentication information
    if let Some(key) = &o.key {
        conn.set_client_cert(o.cert.as_deref(), key)?;
    }
    if let Some(jid) = &o.jid {
        conn.set_jid(jid)?;
    }
    if let Some(password) = &o.password {
        conn.set_pass(password)?;
    }

    if o.certfail {
        conn.set_certfail_handler(certfail_handler);
    }
    if let Some(capath) = &o.capath {
        conn.set_capath(capath)?;
    }
    if let Some(cafile) = &o.cafile {
        conn.set_cafile(cafile)?;
    }

    // initiate connection
    if conn.connect_client(host, port, conn_handler).is_ok() {
        // enter the event loop - our connect handler will trigger an exit
        ctx.run();
    }
    Ok(())
}
