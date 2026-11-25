use super::error::{cstr, opt_cstr, opt_ptr, take_string};
use super::opaque;
use crate::ffi;
use std::ffi::c_ulong;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::Once;
use std::time::Duration;

/// Log level for libstrophe's built-in stderr logger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl From<LogLevel> for ffi::xmpp_log_level_t {
    fn from(l: LogLevel) -> Self {
        match l {
            LogLevel::Debug => ffi::xmpp_log_level_t::XMPP_LEVEL_DEBUG,
            LogLevel::Info => ffi::xmpp_log_level_t::XMPP_LEVEL_INFO,
            LogLevel::Warn => ffi::xmpp_log_level_t::XMPP_LEVEL_WARN,
            LogLevel::Error => ffi::xmpp_log_level_t::XMPP_LEVEL_ERROR,
        }
    }
}

pub(crate) fn millis(d: Duration) -> c_ulong {
    d.as_millis().min(c_ulong::MAX as u128) as c_ulong
}

opaque!(
    /// Borrowed libstrophe context (`xmpp_ctx_t`).
    ContextRef,
    ffi::xmpp_ctx_t
);

impl ContextRef {
    /// Run the event loop until [`ContextRef::stop`] is called.
    pub fn run(&self) {
        unsafe { ffi::xmpp_run(self.as_ptr()) }
    }

    /// Run one event loop iteration, waiting at most `timeout` for I/O.
    pub fn run_once(&self, timeout: Duration) {
        unsafe { ffi::xmpp_run_once(self.as_ptr(), millis(timeout)) }
    }

    /// Make [`ContextRef::run`] return.
    pub fn stop(&self) {
        unsafe { ffi::xmpp_stop(self.as_ptr()) }
    }

    /// Default event loop timeout used by [`ContextRef::run`].
    pub fn set_timeout(&self, timeout: Duration) {
        unsafe { ffi::xmpp_ctx_set_timeout(self.as_ptr(), millis(timeout)) }
    }

    /// Debug verbosity (0 = normal; higher dumps more traffic).
    pub fn set_verbosity(&self, level: i32) {
        unsafe { ffi::xmpp_ctx_set_verbosity(self.as_ptr(), level) }
    }

    unsafe fn take(&self, p: *mut std::ffi::c_char) -> Option<String> {
        take_string(self.as_ptr(), p)
    }

    /// Random UUID v4 string.
    pub fn uuid_gen(&self) -> Option<String> {
        unsafe { self.take(ffi::xmpp_uuid_gen(self.as_ptr())) }
    }

    /// Build `node@domain/resource`.
    pub fn jid_new(&self, node: Option<&str>, domain: &str, resource: Option<&str>) -> Option<String> {
        let (n, d, r) = (opt_cstr(node).ok()?, cstr(domain).ok()?, opt_cstr(resource).ok()?);
        unsafe { self.take(ffi::xmpp_jid_new(self.as_ptr(), opt_ptr(&n), d.as_ptr(), opt_ptr(&r))) }
    }

    pub fn jid_bare(&self, jid: &str) -> Option<String> {
        let j = cstr(jid).ok()?;
        unsafe { self.take(ffi::xmpp_jid_bare(self.as_ptr(), j.as_ptr())) }
    }

    pub fn jid_node(&self, jid: &str) -> Option<String> {
        let j = cstr(jid).ok()?;
        unsafe { self.take(ffi::xmpp_jid_node(self.as_ptr(), j.as_ptr())) }
    }

    pub fn jid_domain(&self, jid: &str) -> Option<String> {
        let j = cstr(jid).ok()?;
        unsafe { self.take(ffi::xmpp_jid_domain(self.as_ptr(), j.as_ptr())) }
    }

    pub fn jid_resource(&self, jid: &str) -> Option<String> {
        let j = cstr(jid).ok()?;
        unsafe { self.take(ffi::xmpp_jid_resource(self.as_ptr(), j.as_ptr())) }
    }

    pub fn base64_encode(&self, data: &[u8]) -> Option<String> {
        unsafe { self.take(ffi::xmpp_base64_encode(self.as_ptr(), data.as_ptr(), data.len())) }
    }

    /// Decode base64 to bytes; `None` on invalid input.
    pub fn base64_decode(&self, b64: &str) -> Option<Vec<u8>> {
        let mut out = std::ptr::null_mut();
        let mut len = 0usize;
        unsafe {
            ffi::xmpp_base64_decode_bin(self.as_ptr(), b64.as_ptr().cast(), b64.len(), &mut out, &mut len);
            if out.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(out, len).to_vec();
            ffi::xmpp_free(self.as_ptr(), out.cast());
            Some(v)
        }
    }
}

/// Owned libstrophe context. Frees the context on drop.
///
/// The library is initialized (`xmpp_initialize`) once per process on first
/// use; `xmpp_shutdown` is never called.
pub struct Context {
    raw: NonNull<ffi::xmpp_ctx_t>,
}

impl Context {
    /// Context with no logging.
    pub fn new() -> Self {
        Self::with_log(std::ptr::null())
    }

    /// Context logging to stderr at `level` and above.
    pub fn with_default_logger(level: LogLevel) -> Self {
        // Points at a static inside libstrophe.
        Self::with_log(unsafe { ffi::xmpp_get_default_logger(level.into()) })
    }

    fn with_log(log: *const ffi::xmpp_log_t) -> Self {
        static INIT: Once = Once::new();
        INIT.call_once(|| unsafe { ffi::xmpp_initialize() });
        let raw = unsafe { ffi::xmpp_ctx_new(std::ptr::null(), log) };
        Context { raw: NonNull::new(raw).expect("xmpp_ctx_new: out of memory") }
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for Context {
    type Target = ContextRef;
    fn deref(&self) -> &ContextRef {
        unsafe { ContextRef::from_ptr(self.raw.as_ptr()) }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { ffi::xmpp_ctx_free(self.raw.as_ptr()) }
    }
}
