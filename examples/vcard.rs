//! Port of libstrophe `examples/vcard.c`: fetch and print someone's vCard.
#![forbid(unsafe_code)]

use libstrophe_vendored::xmpp::{Conn, ConnEvent, Connection, Context, EventKind, LogLevel, StanzaRef};
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

struct Vcard {
    recipient: String,
    img_path: String,
}

type VcardCb = fn(&Vcard, &StanzaRef);

const REQ_TIMEOUT: Duration = Duration::from_millis(5000);

fn vcard_photo(vc: &Vcard, stanza: &StanzaRef) {
    let Some(kind) = stanza.child_by_name("TYPE").and_then(|t| t.text()) else {
        eprintln!("PHOTO without TYPE");
        return;
    };
    println!("PHOTO: {kind}, saving to file {}", vc.img_path);

    let Some(binval) = stanza.child_by_name("BINVAL").and_then(|t| t.text()) else {
        eprintln!("PHOTO without BINVAL");
        return;
    };
    // remove \n and \r
    let copy: String = binval.split(['\n', '\r']).collect();
    let Some(img) = stanza.context().base64_decode(&copy) else {
        eprintln!("PHOTO: invalid base64");
        return;
    };
    if std::fs::write(&vc.img_path, img).is_err() {
        println!("Saving photo failed");
    }
}

fn vcard_print_string(stanza: &StanzaRef, info: &str) {
    println!("{info}: {}", stanza.text().unwrap_or_default());
}

fn vcard_bday(_: &Vcard, stanza: &StanzaRef) {
    vcard_print_string(stanza, "Birthday");
}

fn vcard_desc(_: &Vcard, stanza: &StanzaRef) {
    vcard_print_string(stanza, "Description");
}

fn vcard_email(_: &Vcard, stanza: &StanzaRef) {
    if let Some(userid) = stanza.child_by_name("USERID") {
        vcard_print_string(userid, "E-mail");
    }
}

fn vcard_fn(_: &Vcard, stanza: &StanzaRef) {
    vcard_print_string(stanza, "Full name");
}

fn vcard_name(_: &Vcard, stanza: &StanzaRef) {
    if let Some(name) = stanza.child_by_name("GIVEN") {
        vcard_print_string(name, "Given name");
    }
    if let Some(family) = stanza.child_by_name("FAMILY") {
        vcard_print_string(family, "Family name");
    }
}

fn vcard_nick(_: &Vcard, stanza: &StanzaRef) {
    vcard_print_string(stanza, "Nickname");
}

fn vcard_url(_: &Vcard, stanza: &StanzaRef) {
    vcard_print_string(stanza, "URL");
}

fn vcard_cb_get(stanza: &StanzaRef) -> Option<VcardCb> {
    const VCARD_TBL: [(&str, VcardCb); 8] = [
        ("PHOTO", vcard_photo),
        ("BDAY", vcard_bday),
        ("DESC", vcard_desc),
        ("EMAIL", vcard_email),
        ("FN", vcard_fn),
        ("N", vcard_name),
        ("NICKNAME", vcard_nick),
        ("URL", vcard_url),
    ];
    let tag = stanza.name()?;
    VCARD_TBL.iter().find(|(t, _)| *t == tag).map(|(_, cb)| *cb)
}

fn timedout(conn: &Conn) -> bool {
    eprintln!("Timeout reached.");
    conn.disconnect();
    false
}

fn recv_vcard(conn: &Conn, stanza: &StanzaRef, vc: &Vcard) -> bool {
    println!("Received vCard.\n");

    if let Some(error) = stanza.child_by_name("error") {
        match error.to_text() {
            Ok(s) => println!("Error returned: {s}."),
            Err(e) => println!("Error returned: <{e}>."),
        }
    } else if let Some(vcard) = stanza.child_by_name("vCard") {
        for child in vcard.children() {
            if let Some(cb) = vcard_cb_get(child) {
                cb(vc, child);
            }
        }
    } else {
        eprintln!("Reply has no vCard element.");
    }

    conn.disconnect();
    false
}

fn send_vcard_req(conn: &Conn, to: &str, id: &str) {
    println!("Requesting vCard from {to}.");
    conn.send_raw(&format!(
        "<iq from='{}' to='{}' type='get' id='{}'><vCard xmlns='vcard-temp'/></iq>",
        conn.bound_jid().unwrap_or(""),
        to,
        id
    ));
}

fn conn_handler(conn: &Conn, event: &ConnEvent<'_>, vc: &Rc<Vcard>) {
    if event.kind == EventKind::Connect {
        send_vcard_req(conn, &vc.recipient, "vc1");
        let vc = vc.clone();
        if let Err(e) = conn.id_handler_add("vc1", move |c, s| recv_vcard(c, s, &vc)) {
            eprintln!("ERROR: {e}");
        }
        conn.timed_handler_add(REQ_TIMEOUT, timedout);
    } else {
        if event.error != 0 {
            eprintln!("Disconnected with error={}.", event.error);
        }
        if let Some(se) = &event.stream_error {
            eprintln!("Stream error type={:?} text={}.", se.kind, se.text.unwrap_or("(null)"));
        }
        conn.context().stop();
    }
}

fn main() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if !(4..=5).contains(&args.len()) {
        let prog = args
            .first()
            .and_then(|a| Path::new(a).file_name())
            .map_or("vcard".into(), |n| n.to_string_lossy());
        print!("Usage: {prog} <login-jid> <password> <recipient-jid> [image-file]\n\n");
        println!(
            "If vCard contains a photo it will be stored to image-file. If you don't \
             provide the image-file default filename will be generated."
        );
        return Ok(ExitCode::FAILURE);
    }

    let vcard = Rc::new(Vcard {
        recipient: args[3].clone(),
        img_path: args.get(4).cloned().unwrap_or_else(|| "vcard.jpg".into()),
    });

    let ctx = Context::with_default_logger(LogLevel::Info);
    let conn = Connection::new(&ctx);
    conn.set_jid(&args[1])?;
    conn.set_pass(&args[2])?;
    conn.connect_client(None, 0, move |c, e| conn_handler(c, e, &vcard))?;
    ctx.run();
    Ok(ExitCode::SUCCESS)
}
