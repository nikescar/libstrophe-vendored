//! BOSH echo bot (async counterpart of `bot.rs`): replies to every chat message
//! with its body; a message saying `quit` ends the session.
//!
//! Shows the outbound channel: the handler queues stanzas with `try_send`, and
//! the BOSH client sends them on its next free request.
#![forbid(unsafe_code)]

mod common;
use common::XmppConfig;
use libstrophe_vendored::xmpp::{bosh::connect_bosh, XmppEvent};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = XmppConfig::from_env_and_args(
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
        None,
        None,
        None,
    );
    let url = config.bosh_url.clone().or_else(|| args.get(2).cloned());
    let (true, Some(url)) = (config.is_valid(), url) else {
        eprintln!("Usage: bosh_echo <jid> <pass> <https-url>  (or XMPP_JID, XMPP_PASS, XMPP_BOSH_URL)");
        std::process::exit(1);
    };

    let (tx, rx) = async_channel::unbounded::<String>();
    smol::block_on(connect_bosh(&url, &config.jid, &config.password, rx, |event| match event {
        XmppEvent::Connect => {
            eprintln!("connected; send me a message ('quit' to stop)");
            tx.try_send("<presence/>".into()).is_ok()
        }
        XmppEvent::Stanza(xml) if xml.starts_with("<message") => {
            let (Some(from), Some(body)) = (attr(&xml, "from"), inner(&xml, "body")) else {
                return true;
            };
            if body.trim() == "quit" {
                return false;
            }
            // `from` and `body` are still XML-escaped, so they can be re-embedded as is.
            let reply = format!("<message to='{from}' type='chat'><body>{body}</body></message>");
            tx.try_send(reply).is_ok()
        }
        XmppEvent::Stanza(_) => true,
        XmppEvent::Error(msg) => {
            eprintln!("ERROR: {msg}");
            false
        }
        XmppEvent::Disconnect => {
            eprintln!("disconnected");
            false
        }
    }))?;
    Ok(())
}

/// shortcut: string scan of the first tag, fine for server-generated stanzas.
fn attr<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let tag = &xml[..xml.find('>')?];
    ['\'', '"'].iter().find_map(|q| {
        let start = tag.find(&format!(" {name}={q}"))? + name.len() + 3;
        Some(&tag[start..start + tag[start..].find(*q)?])
    })
}

fn inner<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = xml.find(&format!("<{name}>"))? + name.len() + 2;
    Some(&xml[open..open + xml[open..].find(&format!("</{name}>"))?])
}
