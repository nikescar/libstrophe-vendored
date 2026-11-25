//! C FFI exports for TLS functions
//!
//! Implements all symbols from c-src/libstrophe/tls.h

use super::{session::TlsSession, cert};
use crate::ffi::xmpp_conn_t;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

// C accessor functions from tls_rustls_glue.c
extern "C" {
    fn xmpp_conn_get_sock(conn: *mut xmpp_conn_t) -> c_int;
    fn xmpp_conn_get_domain(conn: *mut xmpp_conn_t) -> *const c_char;
    fn xmpp_conn_get_tls(conn: *mut xmpp_conn_t) -> *mut tls_t;
    fn xmpp_conn_set_tls(conn: *mut xmpp_conn_t, tls: *mut tls_t);
}

/// Opaque TLS session type for C
#[repr(C)]
pub struct tls_t {
    _private: [u8; 0],
}

/// conn_interface from libstrophe common.h
#[repr(C)]
pub struct ConnInterface {
    read: *const c_void,
    write: *const c_void,
    flush: *const c_void,
    pending: *const c_void,
    get_error: *const c_void,
    error_is_recoverable: *const c_void,
    conn: *mut xmpp_conn_t,
}

/// _xmpp_conn_t from libstrophe common.h (partial, only fields we need)
#[repr(C)]
struct XmppConn {
    intf: ConnInterface,
    _padding: [u8; 256], // Skip to fields we need
}

// No helper functions needed - using C accessors directly

/// Initialize TLS backend
#[no_mangle]
pub extern "C" fn tls_initialize() {
    // rustls doesn't need global initialization
}

/// Shutdown TLS backend
#[no_mangle]
pub extern "C" fn tls_shutdown() {
    // rustls doesn't need global cleanup
}

/// Create new TLS session
#[no_mangle]
pub extern "C" fn tls_new(conn: *mut xmpp_conn_t) -> *mut tls_t {
    if conn.is_null() {
        return ptr::null_mut();
    }

    // Get socket fd from connection
    let sock = unsafe { xmpp_conn_get_sock(conn) };
    if sock < 0 {
        return ptr::null_mut();
    }

    // Get domain from connection
    let domain_ptr = unsafe { xmpp_conn_get_domain(conn) };
    if domain_ptr.is_null() {
        return ptr::null_mut();
    }

    let domain = unsafe {
        match CStr::from_ptr(domain_ptr).to_str() {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        }
    };

    // Create TLS session
    match TlsSession::new(sock, domain) {
        Ok(session) => {
            let boxed = Box::new(session);
            Box::into_raw(boxed) as *mut tls_t
        }
        Err(_) => ptr::null_mut(),
    }
}

/// Free TLS session
#[no_mangle]
pub extern "C" fn tls_free(tls: *mut tls_t) {
    if !tls.is_null() {
        unsafe {
            let _ = Box::from_raw(tls as *mut TlsSession);
        }
    }
}

/// Start TLS handshake
#[no_mangle]
pub extern "C" fn tls_start(tls: *mut tls_t) -> c_int {
    if tls.is_null() {
        return -1;
    }

    let session = unsafe { &mut *(tls as *mut TlsSession) };
    session.start()
}

/// Stop TLS session
#[no_mangle]
pub extern "C" fn tls_stop(_tls: *mut tls_t) -> c_int {
    // rustls doesn't need explicit close
    0
}

/// Check if data pending
#[no_mangle]
pub extern "C" fn tls_pending(intf: *mut ConnInterface) -> c_int {
    if intf.is_null() {
        return 0;
    }

    let conn = unsafe { (*intf).conn };
    if conn.is_null() {
        return 0;
    }

    let tls = unsafe { xmpp_conn_get_tls(conn) };
    if tls.is_null() {
        return 0;
    }

    let session = unsafe { &*(tls as *const TlsSession) };
    if session.pending() { 1 } else { 0 }
}

/// Read from TLS session
#[no_mangle]
pub extern "C" fn tls_read(intf: *mut ConnInterface, buff: *mut c_void, len: usize) -> c_int {
    if intf.is_null() || buff.is_null() {
        return -1;
    }

    let conn = unsafe { (*intf).conn };
    if conn.is_null() {
        return -1;
    }

    let tls = unsafe { xmpp_conn_get_tls(conn) };
    if tls.is_null() {
        return -1;
    }

    let session = unsafe { &mut *(tls as *mut TlsSession) };
    let buf = unsafe { std::slice::from_raw_parts_mut(buff as *mut u8, len) };

    session.read(buf) as c_int
}

/// Write to TLS session
#[no_mangle]
pub extern "C" fn tls_write(intf: *mut ConnInterface, buff: *const c_void, len: usize) -> c_int {
    if intf.is_null() || buff.is_null() {
        return -1;
    }

    let conn = unsafe { (*intf).conn };
    if conn.is_null() {
        return -1;
    }

    let tls = unsafe { xmpp_conn_get_tls(conn) };
    if tls.is_null() {
        return -1;
    }

    let session = unsafe { &mut *(tls as *mut TlsSession) };
    let buf = unsafe { std::slice::from_raw_parts(buff as *const u8, len) };

    session.write(buf) as c_int
}

/// Clear pending write (no-op for rustls)
#[no_mangle]
pub extern "C" fn tls_clear_pending_write(_intf: *mut ConnInterface) -> c_int {
    0
}

/// Get last error
#[no_mangle]
pub extern "C" fn tls_error(intf: *mut ConnInterface) -> c_int {
    if intf.is_null() {
        return 0;
    }

    let conn = unsafe { (*intf).conn };
    if conn.is_null() {
        return 0;
    }

    let tls = unsafe { xmpp_conn_get_tls(conn) };
    if tls.is_null() {
        return 0;
    }

    let session = unsafe { &*(tls as *const TlsSession) };
    session.last_error()
}

/// Check if error is recoverable
#[no_mangle]
pub extern "C" fn tls_is_recoverable(_intf: *mut ConnInterface, error: c_int) -> c_int {
    use libc::{EAGAIN, EINTR};
    // EWOULDBLOCK == EAGAIN on most systems
    if error == EAGAIN || error == EINTR {
        1
    } else {
        0
    }
}

// Phase 2 functions (certificate validation)

/// Get XMPP address from peer certificate SAN
#[no_mangle]
pub extern "C" fn tls_id_on_xmppaddr(conn: *mut xmpp_conn_t, n: u32) -> *mut c_char {
    if conn.is_null() {
        return ptr::null_mut();
    }

    let tls = unsafe { xmpp_conn_get_tls(conn) };
    if tls.is_null() {
        return ptr::null_mut();
    }

    let session = unsafe { &*(tls as *const TlsSession) };

    match session.xmpp_addr(n) {
        Some(cstring) => {
            // Clone the CString and leak it - C will manage the lifetime
            cert::leak_cstring(cstring.clone())
        }
        None => ptr::null_mut(),
    }
}

/// Get number of XMPP addresses in peer certificate
#[no_mangle]
pub extern "C" fn tls_id_on_xmppaddr_num(conn: *mut xmpp_conn_t) -> u32 {
    if conn.is_null() {
        return 0;
    }

    let tls = unsafe { xmpp_conn_get_tls(conn) };
    if tls.is_null() {
        return 0;
    }

    let session = unsafe { &*(tls as *const TlsSession) };
    session.xmpp_addr_count()
}

#[no_mangle]
pub extern "C" fn tls_peer_cert(_conn: *mut xmpp_conn_t) -> *mut c_void {
    ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn tls_set_credentials(_tls: *mut tls_t, _cafilename: *const c_char) -> c_int {
    0 // Success - we use webpki-roots by default
}

// Phase 3 functions (channel binding)

/// Initialize channel binding (tls-server-end-point)
#[no_mangle]
pub extern "C" fn tls_init_channel_binding(
    tls: *mut tls_t,
    binding_prefix: *mut *const c_char,
    binding_prefix_len: *mut usize,
) -> c_int {
    if tls.is_null() {
        return -1;
    }

    let session = unsafe { &mut *(tls as *mut TlsSession) };

    if session.init_channel_binding() < 0 {
        return -1;
    }

    // Set prefix to "tls-server-end-point"
    if !binding_prefix.is_null() {
        unsafe {
            *binding_prefix = b"tls-server-end-point\0".as_ptr() as *const c_char;
        }
    }

    if !binding_prefix_len.is_null() {
        unsafe {
            *binding_prefix_len = 20; // "tls-server-end-point" length
        }
    }

    0
}

/// Get channel binding data (SHA-256 hash of server cert)
#[no_mangle]
pub extern "C" fn tls_get_channel_binding_data(tls: *mut tls_t, size: *mut usize) -> *const c_void {
    if tls.is_null() {
        return ptr::null();
    }

    let session = unsafe { &*(tls as *const TlsSession) };

    match session.channel_binding_data() {
        Some((_, data)) => {
            if !size.is_null() {
                unsafe {
                    *size = data.len();
                }
            }
            data.as_ptr() as *const c_void
        }
        None => ptr::null(),
    }
}
