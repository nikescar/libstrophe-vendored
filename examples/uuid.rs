//! Port of libstrophe `examples/uuid.c`: print a random UUID.
#![forbid(unsafe_code)]

use libstrophe_vendored::xmpp::Context;
use std::process::ExitCode;

fn main() -> ExitCode {
    let ctx = Context::new();
    match ctx.uuid_gen() {
        Some(uuid) => {
            println!("{uuid}");
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("Couldn't allocate memory.");
            ExitCode::FAILURE
        }
    }
}
