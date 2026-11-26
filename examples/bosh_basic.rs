//! BOSH port of `basic.rs`: connect over XEP-0124/XEP-0206, authenticate, then disconnect.
//!
//! Flow: session creation, SASL PLAIN, `xmpp:restart`, resource binding,
//! `Connect` event, then `type='terminate'` with unavailable presence.
//!
//! SASL PLAIN needs TLS: use an `https://` URL. `http://` is refused.
#![forbid(unsafe_code)]

mod common;
use common::XmppConfig;
use libstrophe_vendored::xmpp::{bosh::connect_bosh, XmppEvent};
use std::process::exit;

fn usage(exit_code: i32) -> ! {
    eprint!(
        "Usage: bosh_basic [options] [<url>]\n\n\
         Options:\n  \
         --jid <jid>              The JID to use to authenticate.\n  \
         --pass <pass>            The password of the JID.\n\n\
         Arguments:\n  \
         <url>                    BOSH URL (e.g., https://xmpp.example.com/http-bind)\n\n\
         Environment variables (priority over CLI):\n  \
         XMPP_JID                 Jabber ID\n  \
         XMPP_PASS                Password\n  \
         XMPP_BOSH_URL            BOSH URL\n"
    );
    exit(exit_code)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (config, url) = parse_args();
    eprintln!("DEBUG: Connecting to {} as {}", url, config.jid);

    // Nothing to send; the handler ends the session after Connect.
    let (_tx, rx) = async_channel::unbounded::<String>();
    smol::block_on(connect_bosh(&url, &config.jid, &config.password, rx, |event| match event {
        XmppEvent::Connect => {
            eprintln!("DEBUG: connected and authenticated (SASL PLAIN + bind complete)");
            false // disconnect immediately, like basic.rs
        }
        XmppEvent::Stanza(xml) => {
            eprintln!("DEBUG: received stanza: {xml}");
            true
        }
        XmppEvent::Disconnect => {
            eprintln!("DEBUG: disconnected");
            false
        }
        XmppEvent::Error(msg) => {
            eprintln!("ERROR: {msg}");
            false
        }
    }))?;
    Ok(())
}

/// `--jid`, `--pass`, trailing URL; environment variables take priority.
fn parse_args() -> (XmppConfig, String) {
    let args: Vec<String> = std::env::args().collect();
    let (mut jid, mut pass, mut url) = (None, None, None);
    let mut i = 1;
    while i < args.len() {
        let has_value = i + 1 < args.len();
        match args[i].as_str() {
            "--help" => usage(0),
            "--jid" if has_value => {
                i += 1;
                jid = Some(args[i].clone());
            }
            "--pass" if has_value => {
                i += 1;
                pass = Some(args[i].clone());
            }
            arg if !arg.starts_with("--") => url = Some(arg.to_owned()),
            _ => {
                eprintln!("Unknown option: {}", args[i]);
                usage(1);
            }
        }
        i += 1;
    }
    let config = XmppConfig::from_env_and_args(jid.as_deref(), pass.as_deref(), None, None, None);
    if !config.is_valid() {
        eprintln!("Error: JID and password required\n");
        usage(1);
    }
    let Some(url) = config.bosh_url.clone().or(url) else {
        eprintln!("Error: BOSH URL required\n");
        usage(1);
    };
    (config, url)
}
