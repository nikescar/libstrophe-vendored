//! Safe Rust wrapper over the libstrophe FFI.
//!
//! Ownership model mirrors `CString`/`CStr`: owned handles ([`Context`],
//! [`Connection`], [`Stanza`], [`TlsCert`]) release their C object on drop and
//! deref to borrowed views ([`ContextRef`], [`Conn`], [`StanzaRef`],
//! [`TlsCertRef`]), which is what callbacks receive. Lifetimes tie every
//! connection and stanza to the [`Context`] it was created from.
//!
//! Callbacks are Rust closures. libstrophe is single-threaded, so none of
//! these types are `Send`/`Sync`.
//!
//! ```no_run
//! use libstrophe_vendored::xmpp::{Connection, Context, EventKind, LogLevel};
//!
//! let ctx = Context::with_default_logger(LogLevel::Debug);
//! let conn = Connection::new(&ctx);
//! conn.set_jid("user@example.com").unwrap();
//! conn.set_pass("secret").unwrap();
//! conn.connect_client(None, 0, |conn, event| match event.kind {
//!     EventKind::Connect => conn.disconnect(),
//!     _ => conn.context().stop(),
//! })
//! .unwrap();
//! ctx.run();
//! ```

mod cert;
mod conn;
mod ctx;
mod error;
mod stanza;

pub use cert::{CertElement, TlsCert, TlsCertRef, CERT_ELEMENTS};
pub use conn::{
    Conn, ConnEvent, ConnFlags, Connection, EventKind, HandlerId, SmState, Socket, StreamError,
    StreamErrorType,
};
pub use ctx::{Context, ContextRef, LogLevel};
pub use error::{Error, Result};
pub use stanza::{Children, Stanza, StanzaRef};

pub const NS_CLIENT: &str = "jabber:client";
pub const NS_COMPONENT: &str = "jabber:component:accept";
pub const NS_STREAMS: &str = "http://etherx.jabber.org/streams";
pub const NS_STREAMS_IETF: &str = "urn:ietf:params:xml:ns:xmpp-streams";
pub const NS_STANZAS_IETF: &str = "urn:ietf:params:xml:ns:xmpp-stanzas";
pub const NS_TLS: &str = "urn:ietf:params:xml:ns:xmpp-tls";
pub const NS_SASL: &str = "urn:ietf:params:xml:ns:xmpp-sasl";
pub const NS_BIND: &str = "urn:ietf:params:xml:ns:xmpp-bind";
pub const NS_SESSION: &str = "urn:ietf:params:xml:ns:xmpp-session";
pub const NS_AUTH: &str = "jabber:iq:auth";
pub const NS_DISCO_INFO: &str = "http://jabber.org/protocol/disco#info";
pub const NS_DISCO_ITEMS: &str = "http://jabber.org/protocol/disco#items";
pub const NS_ROSTER: &str = "jabber:iq:roster";
pub const NS_REGISTER: &str = "jabber:iq:register";
pub const NS_SM: &str = "urn:xmpp:sm:3";

/// Declares an opaque type only ever used behind `&` (pointer to a C object).
macro_rules! opaque {
    ($(#[$m:meta])* $name:ident, $ffi:ty) => {
        $(#[$m])*
        #[repr(C)]
        pub struct $name {
            _opaque: [u8; 0],
            _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
        }
        impl $name {
            /// # Safety
            /// `p` must be non-null and valid for `'a`.
            #[allow(dead_code)]
            pub(crate) unsafe fn from_ptr<'a>(p: *const $ffi) -> &'a Self {
                debug_assert!(!p.is_null());
                &*(p as *const Self)
            }
            /// # Safety
            /// `p` must be null or valid for `'a`.
            #[allow(dead_code)]
            pub(crate) unsafe fn from_ptr_opt<'a>(p: *const $ffi) -> Option<&'a Self> {
                (p as *const Self).as_ref()
            }
            /// Raw pointer for use with [`crate::ffi`].
            pub fn as_ptr(&self) -> *mut $ffi {
                self as *const Self as *mut $ffi
            }
        }
    };
}
pub(crate) use opaque;

/// Abort instead of unwinding into C (UB before Rust 1.81).
pub(crate) fn no_unwind<R>(f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("libstrophe-vendored: panic in libstrophe callback, aborting");
            std::process::abort()
        }
    }
}
