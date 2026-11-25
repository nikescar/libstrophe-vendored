//! Integration tests for rustls TLS backend
//!
//! Tests the full TLS implementation without needing a live server

use libstrophe_vendored::ffi;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

// TLS functions are exported as C symbols, declare them here
extern "C" {
    fn tls_id_on_xmppaddr(conn: *mut ffi::xmpp_conn_t, n: u32) -> *mut c_char;
    fn tls_id_on_xmppaddr_num(conn: *mut ffi::xmpp_conn_t) -> u32;
    fn tls_is_recoverable(intf: *mut c_void, error: c_int) -> c_int;
}

#[test]
fn test_tls_backend_lifecycle() {
    unsafe {
        // Initialize
        ffi::xmpp_initialize();

        // Create context
        let ctx = ffi::xmpp_ctx_new(ptr::null_mut(), ptr::null_mut());
        assert!(!ctx.is_null(), "Context creation failed");

        // Create connection
        let conn = ffi::xmpp_conn_new(ctx);
        assert!(!conn.is_null(), "Connection creation failed");

        // Set JID (required for connection)
        let jid = CString::new("test@example.com").unwrap();
        ffi::xmpp_conn_set_jid(conn, jid.as_ptr());

        // Verify TLS functions exist and can be called
        // (they will fail without actual connection, but should not crash)

        // Certificate functions should return 0/NULL when no connection
        let num_addrs = tls_id_on_xmppaddr_num(conn);
        assert_eq!(num_addrs, 0, "Should have no XMPP addresses without connection");

        let addr = tls_id_on_xmppaddr(conn, 0);
        assert!(addr.is_null(), "Should return NULL without connection");

        // Cleanup
        ffi::xmpp_conn_release(conn);
        ffi::xmpp_ctx_free(ctx);
        ffi::xmpp_shutdown();
    }
}

#[test]
fn test_tls_error_handling() {
    unsafe {
        ffi::xmpp_initialize();

        let ctx = ffi::xmpp_ctx_new(ptr::null_mut(), ptr::null_mut());
        assert!(!ctx.is_null());

        let conn = ffi::xmpp_conn_new(ctx);
        assert!(!conn.is_null());

        // Test NULL safety - should not crash
        assert_eq!(tls_id_on_xmppaddr_num(ptr::null_mut()), 0);
        assert!(tls_id_on_xmppaddr(ptr::null_mut(), 0).is_null());

        // Test tls_is_recoverable with various errors
        let recoverable_errors = [libc::EAGAIN, libc::EINTR];
        for &err in &recoverable_errors {
            let result = tls_is_recoverable(ptr::null_mut(), err);
            assert_eq!(result, 1, "Error {} should be recoverable", err);
        }

        let fatal_error = libc::ECONNRESET;
        let result = tls_is_recoverable(ptr::null_mut(), fatal_error);
        assert_eq!(result, 0, "ECONNRESET should not be recoverable");

        ffi::xmpp_conn_release(conn);
        ffi::xmpp_ctx_free(ctx);
        ffi::xmpp_shutdown();
    }
}

#[test]
fn test_context_multiple_connections() {
    unsafe {
        ffi::xmpp_initialize();

        let ctx = ffi::xmpp_ctx_new(ptr::null_mut(), ptr::null_mut());
        assert!(!ctx.is_null());

        // Create multiple connections
        let conn1 = ffi::xmpp_conn_new(ctx);
        let conn2 = ffi::xmpp_conn_new(ctx);
        let conn3 = ffi::xmpp_conn_new(ctx);

        assert!(!conn1.is_null());
        assert!(!conn2.is_null());
        assert!(!conn3.is_null());

        // All should be independent
        assert_ne!(conn1, conn2);
        assert_ne!(conn2, conn3);
        assert_ne!(conn1, conn3);

        // Set different JIDs
        let jid1 = CString::new("user1@example.com").unwrap();
        let jid2 = CString::new("user2@example.com").unwrap();
        let jid3 = CString::new("user3@example.com").unwrap();

        ffi::xmpp_conn_set_jid(conn1, jid1.as_ptr());
        ffi::xmpp_conn_set_jid(conn2, jid2.as_ptr());
        ffi::xmpp_conn_set_jid(conn3, jid3.as_ptr());

        // Cleanup in reverse order
        ffi::xmpp_conn_release(conn3);
        ffi::xmpp_conn_release(conn2);
        ffi::xmpp_conn_release(conn1);
        ffi::xmpp_ctx_free(ctx);
        ffi::xmpp_shutdown();
    }
}

#[test]
fn test_tls_flags() {
    unsafe {
        ffi::xmpp_initialize();

        let ctx = ffi::xmpp_ctx_new(ptr::null_mut(), ptr::null_mut());
        let conn = ffi::xmpp_conn_new(ctx);

        // Test TLS flag constants exist
        const DISABLE_TLS: i64 = 1; // XMPP_CONN_FLAG_DISABLE_TLS
        const MANDATORY_TLS: i64 = 2; // XMPP_CONN_FLAG_MANDATORY_TLS
        const TRUST_TLS: i64 = 8; // XMPP_CONN_FLAG_TRUST_TLS

        // Should be able to set flags
        ffi::xmpp_conn_set_flags(conn, MANDATORY_TLS);

        // Can combine flags
        ffi::xmpp_conn_set_flags(conn, MANDATORY_TLS | TRUST_TLS);

        ffi::xmpp_conn_release(conn);
        ffi::xmpp_ctx_free(ctx);
        ffi::xmpp_shutdown();
    }
}
