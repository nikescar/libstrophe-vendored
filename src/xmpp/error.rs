use crate::ffi;
use std::ffi::{c_char, c_int, CStr, CString};
use std::fmt;

/// Errors reported by libstrophe or by argument conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// `XMPP_EMEM`: allocation failed.
    Mem,
    /// `XMPP_EINVOP`: operation invalid in current state.
    InvalidOp,
    /// `XMPP_EINT`: internal error.
    Internal,
    /// A string argument contained an interior NUL byte.
    Nul,
    /// libstrophe returned NULL (e.g. XML parse failure).
    Null,
    /// Any other non-zero return code.
    Code(i32),
    /// Network I/O failed (connect, read, write, timeout).
    Io,
    /// TLS handshake or certificate verification failed.
    Tls,
    /// Authentication failed or was refused (e.g. SASL `<failure/>`, PLAIN without TLS).
    Auth,
    /// Peer violated the wire protocol (bad URL, HTTP status, BOSH/XMPP framing).
    Protocol,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Mem => f.write_str("out of memory"),
            Error::InvalidOp => f.write_str("invalid operation"),
            Error::Internal => f.write_str("internal error"),
            Error::Nul => f.write_str("string contains NUL byte"),
            Error::Null => f.write_str("libstrophe returned NULL"),
            Error::Code(c) => write!(f, "libstrophe error code {c}"),
            Error::Io => f.write_str("network I/O error"),
            Error::Tls => f.write_str("TLS handshake or certificate verification failed"),
            Error::Auth => f.write_str("authentication failed"),
            Error::Protocol => f.write_str("protocol error"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn check(rc: c_int) -> Result<()> {
    match rc {
        ffi::XMPP_EOK => Ok(()),
        ffi::XMPP_EMEM => Err(Error::Mem),
        ffi::XMPP_EINVOP => Err(Error::InvalidOp),
        ffi::XMPP_EINT => Err(Error::Internal),
        c => Err(Error::Code(c)),
    }
}

pub(crate) fn cstr(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::Nul)
}

pub(crate) fn opt_cstr(s: Option<&str>) -> Result<Option<CString>> {
    s.map(cstr).transpose()
}

pub(crate) fn opt_ptr(s: &Option<CString>) -> *const c_char {
    s.as_ref().map_or(std::ptr::null(), |s| s.as_ptr())
}

/// Borrow a C string. Non-UTF-8 yields `None` (expat always produces UTF-8).
///
/// # Safety
/// `p` must be null or a valid NUL-terminated string living for `'a`.
pub(crate) unsafe fn borrow_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        None
    } else {
        CStr::from_ptr(p).to_str().ok()
    }
}

/// Copy a libstrophe-allocated string and free it with `xmpp_free`.
///
/// # Safety
/// `p` must be null or a string allocated by `ctx`.
pub(crate) unsafe fn take_string(ctx: *const ffi::xmpp_ctx_t, p: *mut c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = CStr::from_ptr(p).to_string_lossy().into_owned();
    ffi::xmpp_free(ctx, p.cast());
    Some(s)
}
