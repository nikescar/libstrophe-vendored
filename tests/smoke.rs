//! Smoke test: Verify vendored libstrophe compiles and links correctly
//!
//! This test validates the C library is properly compiled and all symbols
//! are accessible from Rust.

use libstrophe_vendored::ffi;

#[test]
fn test_xmpp_initialize_and_shutdown() {
    // Act: Call library initialization and shutdown
    unsafe {
        ffi::xmpp_initialize();
        ffi::xmpp_shutdown();
    }

    // Assert: No crash = success
}

#[test]
fn test_context_lifecycle() {
    // Arrange
    unsafe { ffi::xmpp_initialize() };

    // Act: Create context with default allocator and logger
    let ctx = unsafe { ffi::xmpp_ctx_new(std::ptr::null_mut(), std::ptr::null_mut()) };

    // Assert: Context created successfully
    assert!(!ctx.is_null(), "xmpp_ctx_new() should return non-null context");

    // Cleanup
    unsafe {
        ffi::xmpp_ctx_free(ctx);
        ffi::xmpp_shutdown();
    }
}

#[test]
fn test_connection_lifecycle() {
    // Arrange
    unsafe { ffi::xmpp_initialize() };
    let ctx = unsafe { ffi::xmpp_ctx_new(std::ptr::null_mut(), std::ptr::null_mut()) };
    assert!(!ctx.is_null());

    // Act: Create connection
    let conn = unsafe { ffi::xmpp_conn_new(ctx) };

    // Assert: Connection created successfully
    assert!(!conn.is_null(), "xmpp_conn_new() should return non-null connection");

    // Cleanup
    unsafe {
        ffi::xmpp_conn_release(conn);
        ffi::xmpp_ctx_free(ctx);
        ffi::xmpp_shutdown();
    }
}
