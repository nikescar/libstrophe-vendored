use super::cert::{TlsCert, TlsCertRef};
use super::ctx::{millis, ContextRef};
use super::error::{borrow_str, check, cstr, opt_cstr, opt_ptr, Error, Result};
use super::no_unwind;
use super::stanza::StanzaRef;
use crate::ffi;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_long, c_ushort, c_void};
use std::marker::PhantomData;
use std::ops::{BitOr, BitOrAssign, Deref};
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Duration;

pub use ffi::xmpp_error_type_t as StreamErrorType;

/// Socket handed to [`Conn::set_sockopt_callback`] callbacks.
#[cfg(unix)]
pub type Socket<'a> = std::os::fd::BorrowedFd<'a>;
/// Socket handed to [`Conn::set_sockopt_callback`] callbacks.
#[cfg(windows)]
pub type Socket<'a> = std::os::windows::io::BorrowedSocket<'a>;

/// Connection flags (`XMPP_CONN_FLAG_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConnFlags(c_long);

impl ConnFlags {
    pub const DISABLE_TLS: Self = Self(ffi::XMPP_CONN_FLAG_DISABLE_TLS);
    pub const MANDATORY_TLS: Self = Self(ffi::XMPP_CONN_FLAG_MANDATORY_TLS);
    pub const LEGACY_SSL: Self = Self(ffi::XMPP_CONN_FLAG_LEGACY_SSL);
    pub const TRUST_TLS: Self = Self(ffi::XMPP_CONN_FLAG_TRUST_TLS);
    pub const LEGACY_AUTH: Self = Self(ffi::XMPP_CONN_FLAG_LEGACY_AUTH);
    pub const DISABLE_SM: Self = Self(ffi::XMPP_CONN_FLAG_DISABLE_SM);
    pub const ENABLE_COMPRESSION: Self = Self(ffi::XMPP_CONN_FLAG_ENABLE_COMPRESSION);
    pub const COMPRESSION_DONT_RESET: Self = Self(ffi::XMPP_CONN_FLAG_COMPRESSION_DONT_RESET);

    pub const fn empty() -> Self {
        Self(0)
    }
    pub const fn bits(self) -> c_long {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for ConnFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for ConnFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Connection event kind (`xmpp_conn_event_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Stream negotiated (client/component), or stream opened (raw).
    Connect,
    /// TCP (+TLS) up for a raw connection; call [`Conn::open_stream_default`].
    RawConnect,
    Disconnect,
    Fail,
}

impl From<ffi::xmpp_conn_event_t> for EventKind {
    fn from(e: ffi::xmpp_conn_event_t) -> Self {
        use ffi::xmpp_conn_event_t::*;
        match e {
            XMPP_CONN_CONNECT => EventKind::Connect,
            XMPP_CONN_RAW_CONNECT => EventKind::RawConnect,
            XMPP_CONN_DISCONNECT => EventKind::Disconnect,
            XMPP_CONN_FAIL => EventKind::Fail,
        }
    }
}

/// `<stream:error/>` received from the server.
#[derive(Debug, Clone, Copy)]
pub struct StreamError<'a> {
    pub kind: StreamErrorType,
    pub text: Option<&'a str>,
    pub stanza: Option<&'a StanzaRef>,
}

/// Argument of the connection handler.
#[derive(Debug, Clone, Copy)]
pub struct ConnEvent<'a> {
    pub kind: EventKind,
    /// Socket-level error code (0 = none).
    pub error: i32,
    pub stream_error: Option<StreamError<'a>>,
}

/// Handle to a registered handler; pass to [`Conn::handler_delete`].
#[derive(Debug, Clone)]
pub struct HandlerId(Rc<Cell<bool>>);

/// Type-erased boxed closure, freed when the [`Connection`] is dropped.
struct Erased {
    ptr: *mut (),
    drop: unsafe fn(*mut ()),
}

impl Drop for Erased {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.ptr) }
    }
}

unsafe fn drop_box<T>(p: *mut ()) {
    drop(Box::from_raw(p as *mut T));
}

/// Userdata passed to libstrophe for every closure-based callback.
struct Slot<F> {
    conn: *const Conn,
    active: Rc<Cell<bool>>,
    f: RefCell<F>,
}

type CertfailFn = dyn FnMut(&TlsCertRef, &str) -> bool;
type SockoptFn = dyn FnMut(&Conn, Socket<'_>) -> std::io::Result<()>;

thread_local! {
    // certfail/sockopt callbacks get no userdata: find our Conn by raw pointer.
    static CONNS: RefCell<HashMap<usize, *const Conn>> = RefCell::new(HashMap::new());
}

fn lookup(raw: *mut ffi::xmpp_conn_t) -> Option<*const Conn> {
    CONNS.with(|m| m.borrow().get(&(raw as usize)).copied())
}

/// Borrowed connection: everything you can do with an `xmpp_conn_t`.
/// Obtained by deref of [`Connection`] or as the first argument of callbacks.
pub struct Conn {
    raw: NonNull<ffi::xmpp_conn_t>,
    // shortcut: closures live until the Connection drops (one-shot handlers
    // included); upgrade to per-slot freeing if long-lived conns churn handlers.
    slots: RefCell<Vec<Erased>>,
    certfail: RefCell<Option<Box<CertfailFn>>>,
    sockopt: RefCell<Option<Box<SockoptFn>>>,
    releasing: Cell<bool>,
}

// ---------------------------------------------------------------- trampolines

unsafe extern "C" fn conn_cb<F>(
    _raw: *mut ffi::xmpp_conn_t,
    event: ffi::xmpp_conn_event_t,
    error: c_int,
    se: *mut ffi::xmpp_stream_error_t,
    ud: *mut c_void,
) where
    F: FnMut(&Conn, &ConnEvent<'_>),
{
    let slot = &*(ud as *const Slot<F>);
    let stream_error = se.as_ref().map(|e| StreamError {
        kind: e.type_,
        text: borrow_str(e.text),
        stanza: StanzaRef::from_ptr_opt(e.stanza),
    });
    let event = ConnEvent { kind: event.into(), error, stream_error };
    no_unwind(|| {
        if let Ok(mut f) = slot.f.try_borrow_mut() {
            f(&*slot.conn, &event)
        }
    });
}

/// Shared logic for handlers whose return value means "keep me".
unsafe fn fire<F>(ud: *mut c_void, call: impl FnOnce(&mut F, &Conn) -> bool) -> c_int {
    let slot = &*(ud as *const Slot<F>);
    if !slot.active.get() {
        return 0; // deleted via handler_delete: let libstrophe drop it now
    }
    let keep = no_unwind(|| match slot.f.try_borrow_mut() {
        Ok(mut f) => call(&mut f, &*slot.conn),
        Err(_) => true, // re-entrant call (nested event loop): skip
    });
    if !keep {
        slot.active.set(false);
    }
    slot.active.get() as c_int
}

unsafe extern "C" fn stanza_cb<F>(_raw: *mut ffi::xmpp_conn_t, st: *mut ffi::xmpp_stanza_t, ud: *mut c_void) -> c_int
where
    F: FnMut(&Conn, &StanzaRef) -> bool,
{
    fire::<F>(ud, |f, c| f(c, StanzaRef::from_ptr(st)))
}

unsafe extern "C" fn timed_cb<F>(_raw: *mut ffi::xmpp_conn_t, ud: *mut c_void) -> c_int
where
    F: FnMut(&Conn) -> bool,
{
    fire::<F>(ud, |f, c| f(c))
}

unsafe extern "C" fn password_cb<F>(pw: *mut c_char, pw_max: usize, _raw: *mut ffi::xmpp_conn_t, ud: *mut c_void) -> c_int
where
    F: FnMut(&Conn, usize) -> Option<String>,
{
    let slot = &*(ud as *const Slot<F>);
    let pass = no_unwind(|| slot.f.try_borrow_mut().ok().and_then(|mut f| f(&*slot.conn, pw_max)));
    let Some(pass) = pass else { return -1 };
    let mut bytes = pass.into_bytes();
    let ret = if bytes.len() < pw_max && !bytes.contains(&0) {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pw.cast::<u8>(), bytes.len());
        *pw.add(bytes.len()) = 0;
        bytes.len() as c_int
    } else {
        -1
    };
    bytes.fill(0);
    ret
}

unsafe extern "C" fn certfail_cb(cert: *const ffi::xmpp_tlscert_t, msg: *const c_char) -> c_int {
    let Some(conn) = lookup(ffi::xmpp_tlscert_get_conn(cert)) else { return 0 };
    let conn = &*conn;
    // Take the closure out so it may replace itself without a RefCell panic.
    let Some(mut f) = conn.certfail.borrow_mut().take() else { return 0 };
    let ok = no_unwind(|| f(TlsCertRef::from_ptr(cert), borrow_str(msg).unwrap_or("")));
    conn.certfail.borrow_mut().get_or_insert(f);
    ok as c_int
}

unsafe extern "C" fn sockopt_cb(raw: *mut ffi::xmpp_conn_t, sock: *mut c_void) -> c_int {
    let Some(conn) = lookup(raw) else { return 0 };
    let conn = &*conn;
    let Some(mut f) = conn.sockopt.borrow_mut().take() else { return 0 };
    #[cfg(unix)]
    let s = std::os::fd::BorrowedFd::borrow_raw(*(sock as *const std::os::fd::RawFd));
    #[cfg(windows)]
    let s = std::os::windows::io::BorrowedSocket::borrow_raw(*(sock as *const usize) as _);
    let r = no_unwind(|| f(conn, s));
    conn.sockopt.borrow_mut().get_or_insert(f);
    if r.is_ok() {
        0
    } else {
        -1
    }
}

type ConnectFn =
    unsafe extern "C" fn(*mut ffi::xmpp_conn_t, *const c_char, c_ushort, ffi::xmpp_conn_handler, *mut c_void) -> c_int;

impl Conn {
    fn p(&self) -> *mut ffi::xmpp_conn_t {
        self.raw.as_ptr()
    }

    fn register<F: 'static>(&self, f: F) -> (*mut c_void, HandlerId) {
        let active = Rc::new(Cell::new(true));
        let slot = Box::into_raw(Box::new(Slot { conn: self, active: active.clone(), f: RefCell::new(f) }));
        self.slots.borrow_mut().push(Erased { ptr: slot.cast(), drop: drop_box::<Slot<F>> });
        (slot.cast(), HandlerId(active))
    }

    fn set_str(&self, f: unsafe extern "C" fn(*mut ffi::xmpp_conn_t, *const c_char), v: &str) -> Result<()> {
        let v = cstr(v)?;
        unsafe { f(self.p(), v.as_ptr()) };
        Ok(())
    }

    pub fn context(&self) -> &ContextRef {
        unsafe { ContextRef::from_ptr(ffi::xmpp_conn_get_context(self.p())) }
    }

    pub fn flags(&self) -> ConnFlags {
        ConnFlags(unsafe { ffi::xmpp_conn_get_flags(self.p()) })
    }

    /// Fails with [`Error::InvalidOp`] unless disconnected.
    pub fn set_flags(&self, flags: ConnFlags) -> Result<()> {
        check(unsafe { ffi::xmpp_conn_set_flags(self.p(), flags.0) })
    }

    pub fn jid(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_conn_get_jid(self.p())) }
    }

    /// JID assigned by the server after resource binding.
    pub fn bound_jid(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_conn_get_bound_jid(self.p())) }
    }

    pub fn set_jid(&self, jid: &str) -> Result<()> {
        self.set_str(ffi::xmpp_conn_set_jid, jid)
    }

    pub fn pass(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_conn_get_pass(self.p())) }
    }

    pub fn set_pass(&self, pass: &str) -> Result<()> {
        self.set_str(ffi::xmpp_conn_set_pass, pass)
    }

    pub fn set_cafile(&self, path: &str) -> Result<()> {
        self.set_str(ffi::xmpp_conn_set_cafile, path)
    }

    pub fn set_capath(&self, path: &str) -> Result<()> {
        self.set_str(ffi::xmpp_conn_set_capath, path)
    }

    /// Client certificate (PEM) and key, or a PKCS#12 file as `key` with `cert = None`.
    pub fn set_client_cert(&self, cert: Option<&str>, key: &str) -> Result<()> {
        let (c, k) = (opt_cstr(cert)?, cstr(key)?);
        unsafe { ffi::xmpp_conn_set_client_cert(self.p(), opt_ptr(&c), k.as_ptr()) };
        Ok(())
    }

    pub fn keyfile(&self) -> Option<&str> {
        unsafe { borrow_str(ffi::xmpp_conn_get_keyfile(self.p())) }
    }

    pub fn set_password_retries(&self, retries: u32) {
        unsafe { ffi::xmpp_conn_set_password_retries(self.p(), retries) }
    }

    /// Called to unlock an encrypted client key: `f(conn, max_len)` returns the
    /// password (must be shorter than `max_len`), or `None` to give up.
    pub fn set_password_callback<F>(&self, f: F)
    where
        F: FnMut(&Conn, usize) -> Option<String> + 'static,
    {
        let (ud, _) = self.register(f);
        unsafe { ffi::xmpp_conn_set_password_callback(self.p(), Some(password_cb::<F>), ud) }
    }

    /// Called when the server certificate can't be verified: return `true` to
    /// accept it anyway.
    pub fn set_certfail_handler<F>(&self, f: F)
    where
        F: FnMut(&TlsCertRef, &str) -> bool + 'static,
    {
        *self.certfail.borrow_mut() = Some(Box::new(f));
        unsafe { ffi::xmpp_conn_set_certfail_handler(self.p(), Some(certfail_cb)) }
    }

    /// Called with the socket right after it is created, to set options.
    pub fn set_sockopt_callback<F>(&self, f: F)
    where
        F: FnMut(&Conn, Socket<'_>) -> std::io::Result<()> + 'static,
    {
        *self.sockopt.borrow_mut() = Some(Box::new(f));
        unsafe { ffi::xmpp_conn_set_sockopt_callback(self.p(), Some(sockopt_cb)) }
    }

    /// Use libstrophe's built-in TCP keepalive socket options.
    pub fn set_sockopt_keepalive(&self) {
        unsafe { ffi::xmpp_conn_set_sockopt_callback(self.p(), Some(ffi::xmpp_sockopt_cb_keepalive)) }
    }

    pub fn is_secured(&self) -> bool {
        unsafe { ffi::xmpp_conn_is_secured(self.p()) != 0 }
    }

    pub fn is_connecting(&self) -> bool {
        unsafe { ffi::xmpp_conn_is_connecting(self.p()) != 0 }
    }

    pub fn is_connected(&self) -> bool {
        unsafe { ffi::xmpp_conn_is_connected(self.p()) != 0 }
    }

    pub fn is_disconnected(&self) -> bool {
        unsafe { ffi::xmpp_conn_is_disconnected(self.p()) != 0 }
    }

    pub fn send_queue_len(&self) -> usize {
        unsafe { ffi::xmpp_conn_send_queue_len(self.p()) }.max(0) as usize
    }

    /// Server certificate of a secured connection.
    pub fn peer_cert(&self) -> Option<TlsCert<'_>> {
        unsafe { TlsCert::from_raw(ffi::xmpp_conn_get_peer_cert(self.p())) }
    }

    fn connect_with<F>(&self, connect: ConnectFn, host: Option<&str>, port: u16, handler: F) -> Result<()>
    where
        F: FnMut(&Conn, &ConnEvent<'_>) + 'static,
    {
        if self.releasing.get() {
            return Err(Error::InvalidOp);
        }
        let h = opt_cstr(host)?;
        let (ud, _) = self.register(handler);
        check(unsafe { connect(self.p(), opt_ptr(&h), port, Some(conn_cb::<F>), ud) })
    }

    /// Connect as a client. `altdomain`/`altport` override SRV lookup (port 0 = default).
    pub fn connect_client<F>(&self, altdomain: Option<&str>, altport: u16, handler: F) -> Result<()>
    where
        F: FnMut(&Conn, &ConnEvent<'_>) + 'static,
    {
        self.connect_with(ffi::xmpp_connect_client, altdomain, altport, handler)
    }

    /// Connect as an external component (XEP-0114). Port 0 = 5347.
    pub fn connect_component<F>(&self, server: &str, port: u16, handler: F) -> Result<()>
    where
        F: FnMut(&Conn, &ConnEvent<'_>) + 'static,
    {
        self.connect_with(ffi::xmpp_connect_component, Some(server), port, handler)
    }

    /// Connect without stream negotiation; the handler gets
    /// [`EventKind::RawConnect`] and drives the stream itself.
    pub fn connect_raw<F>(&self, altdomain: Option<&str>, altport: u16, handler: F) -> Result<()>
    where
        F: FnMut(&Conn, &ConnEvent<'_>) + 'static,
    {
        self.connect_with(ffi::xmpp_connect_raw, altdomain, altport, handler)
    }

    pub fn open_stream_default(&self) -> Result<()> {
        check(unsafe { ffi::xmpp_conn_open_stream_default(self.p()) })
    }

    /// Start TLS on a raw connection (after `<proceed/>`).
    pub fn tls_start(&self) -> Result<()> {
        check(unsafe { ffi::xmpp_conn_tls_start(self.p()) })
    }

    pub fn disconnect(&self) {
        unsafe { ffi::xmpp_disconnect(self.p()) }
    }

    pub fn send(&self, stanza: &StanzaRef) {
        unsafe { ffi::xmpp_send(self.p(), stanza.as_ptr()) }
    }

    /// Send raw XML data as-is.
    pub fn send_raw(&self, data: &str) {
        unsafe { ffi::xmpp_send_raw(self.p(), data.as_ptr().cast(), data.len()) }
    }

    /// Stanza handler matching `ns` (of the element or its first child),
    /// element `name` and `type` attribute; `None` matches anything.
    /// Return `true` to stay registered.
    pub fn handler_add<F>(&self, ns: Option<&str>, name: Option<&str>, type_: Option<&str>, f: F) -> Result<HandlerId>
    where
        F: FnMut(&Conn, &StanzaRef) -> bool + 'static,
    {
        let (n, m, t) = (opt_cstr(ns)?, opt_cstr(name)?, opt_cstr(type_)?);
        let (ud, id) = self.register(f);
        unsafe { ffi::xmpp_handler_add(self.p(), Some(stanza_cb::<F>), opt_ptr(&n), opt_ptr(&m), opt_ptr(&t), ud) };
        Ok(id)
    }

    /// Handler for stanzas with attribute `id`. Return `true` to stay registered.
    pub fn id_handler_add<F>(&self, id: &str, f: F) -> Result<HandlerId>
    where
        F: FnMut(&Conn, &StanzaRef) -> bool + 'static,
    {
        let i = cstr(id)?;
        let (ud, hid) = self.register(f);
        unsafe { ffi::xmpp_id_handler_add(self.p(), Some(stanza_cb::<F>), i.as_ptr(), ud) };
        Ok(hid)
    }

    /// Called every `period` while connected. Return `true` to keep firing.
    pub fn timed_handler_add<F>(&self, period: Duration, f: F) -> HandlerId
    where
        F: FnMut(&Conn) -> bool + 'static,
    {
        let (ud, id) = self.register(f);
        unsafe { ffi::xmpp_timed_handler_add(self.p(), Some(timed_cb::<F>), millis(period), ud) };
        id
    }

    /// Unregister a handler of any kind. It never fires again.
    // libstrophe's *_handler_delete match on function pointer only (would hit
    // every closure of the same type), so the slot is disabled and libstrophe
    // drops it the next time it would fire.
    pub fn handler_delete(&self, id: &HandlerId) {
        id.0.set(false);
    }
}

/// Stream Management state carried from one connection to the next.
pub struct SmState<'cx> {
    raw: NonNull<ffi::xmpp_sm_state_t>,
    _ctx: PhantomData<&'cx ContextRef>,
}

impl Drop for SmState<'_> {
    fn drop(&mut self) {
        unsafe { ffi::xmpp_free_sm_state(self.raw.as_ptr()) }
    }
}

/// Owned connection (`xmpp_conn_t`), released on drop. Derefs to [`Conn`].
pub struct Connection<'cx> {
    // Raw (not Box) so moving the Connection never invalidates the pointers
    // held by registered callbacks.
    inner: NonNull<Conn>,
    _ctx: PhantomData<&'cx ContextRef>,
}

impl<'cx> Connection<'cx> {
    pub fn new(ctx: &'cx ContextRef) -> Self {
        let raw = NonNull::new(unsafe { ffi::xmpp_conn_new(ctx.as_ptr()) }).expect("xmpp_conn_new: out of memory");
        let inner = Box::into_raw(Box::new(Conn {
            raw,
            slots: RefCell::new(Vec::new()),
            certfail: RefCell::new(None),
            sockopt: RefCell::new(None),
            releasing: Cell::new(false),
        }));
        CONNS.with(|m| m.borrow_mut().insert(raw.as_ptr() as usize, inner));
        Connection { inner: unsafe { NonNull::new_unchecked(inner) }, _ctx: PhantomData }
    }

    /// Take the Stream Management state (only while disconnected).
    pub fn take_sm_state(&self) -> Option<SmState<'cx>> {
        NonNull::new(unsafe { ffi::xmpp_conn_get_sm_state(self.p()) }).map(|raw| SmState { raw, _ctx: PhantomData })
    }

    /// Resume with a previous connection's state (only while disconnected).
    pub fn set_sm_state(&self, state: SmState<'cx>) -> Result<()> {
        check(unsafe { ffi::xmpp_conn_set_sm_state(self.p(), state.raw.as_ptr()) })?;
        std::mem::forget(state); // owned by the connection now
        Ok(())
    }
}

impl Deref for Connection<'_> {
    type Target = Conn;
    fn deref(&self) -> &Conn {
        unsafe { self.inner.as_ref() }
    }
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        let raw = self.p();
        self.releasing.set(true);
        // May fire the disconnect handler, so closures must still be alive.
        unsafe { ffi::xmpp_conn_release(raw) };
        CONNS.with(|m| m.borrow_mut().remove(&(raw as usize)));
        drop(unsafe { Box::from_raw(self.inner.as_ptr()) });
    }
}
