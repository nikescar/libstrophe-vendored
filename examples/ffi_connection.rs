//! Raw FFI XMPP connection example with rustls TLS
//!
//! Usage: cargo run --example ffi_connection --features tls-rustls
//!
//! This demonstrates the low-level FFI API (compare with safe_connection.rs):
//! - Direct FFI calls to libstrophe
//! - Manual memory management (unsafe blocks)
//! - Function pointer callbacks
//! - Raw pointer handling

use libstrophe_vendored::ffi;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

/// Connection event callback
unsafe extern "C" fn conn_handler(
    _conn: *mut ffi::xmpp_conn_t,
    event: ffi::xmpp_conn_event_t,
    _error: c_int,
    _stream_error: *mut ffi::xmpp_stream_error_t,
    _userdata: *mut c_void,
) {
    match event {
        ffi::xmpp_conn_event_t::XMPP_CONN_CONNECT => {
            println!("Connected!");
        }
        ffi::xmpp_conn_event_t::XMPP_CONN_DISCONNECT => {
            println!("Disconnected!");
        }
        ffi::xmpp_conn_event_t::XMPP_CONN_FAIL => {
            println!("Connection failed!");
        }
        _ => {}
    }
}

fn main() {
    println!("libstrophe-vendored basic connection example");
    println!("============================================\n");

    // Get connection details from environment or use defaults
    let jid = std::env::var("XMPP_JID").unwrap_or_else(|_| "test@jabber.org".to_string());
    let pass = std::env::var("XMPP_PASS").unwrap_or_else(|_| "password".to_string());
    let host = std::env::var("XMPP_HOST").ok();

    println!("Connecting as: {}", jid);
    if let Some(ref h) = host {
        println!("Server: {}", h);
    }
    println!();

    unsafe {
        // Initialize libstrophe
        ffi::xmpp_initialize();

        // Create context with default logger
        let ctx = ffi::xmpp_ctx_new(ptr::null_mut(), ptr::null_mut());
        if ctx.is_null() {
            eprintln!("Failed to create context");
            return;
        }

        // Create connection
        let conn = ffi::xmpp_conn_new(ctx);
        if conn.is_null() {
            eprintln!("Failed to create connection");
            ffi::xmpp_ctx_free(ctx);
            return;
        }

        // Set JID and password
        let jid_c = CString::new(jid.as_str()).unwrap();
        let pass_c = CString::new(pass.as_str()).unwrap();

        ffi::xmpp_conn_set_jid(conn, jid_c.as_ptr());
        ffi::xmpp_conn_set_pass(conn, pass_c.as_ptr());

        // Enable TLS (required for rustls backend)
        // XMPP_CONN_FLAG_MANDATORY_TLS = 2
        ffi::xmpp_conn_set_flags(conn, 2);

        println!("Initiating connection...");

        // Connect (with optional custom host)
        let host_ptr = host
            .as_ref()
            .map(|h| CString::new(h.as_str()).unwrap())
            .map(|cs| cs.as_ptr())
            .unwrap_or(ptr::null());

        let result = ffi::xmpp_connect_client(
            conn,
            host_ptr,
            0, // default port
            Some(conn_handler),
            ptr::null_mut(),
        );

        if result < 0 {
            eprintln!("Connection failed to start (error: {})", result);
            ffi::xmpp_conn_release(conn);
            ffi::xmpp_ctx_free(ctx);
            return;
        }

        println!("Running event loop (Ctrl+C to stop)...\n");

        // Run event loop for 30 seconds
        for i in 0..30 {
            ffi::xmpp_run_once(ctx, 1000); // 1 second timeout

            if i % 5 == 0 {
                println!("Tick {}s", i);
            }
        }

        println!("\nShutting down...");

        // Cleanup
        ffi::xmpp_disconnect(conn);
        ffi::xmpp_conn_release(conn);
        ffi::xmpp_ctx_free(ctx);
        ffi::xmpp_shutdown();

        println!("Done!");
    }
}

// TLS functions (exported as C symbols)
extern "C" {
    fn tls_id_on_xmppaddr(conn: *mut ffi::xmpp_conn_t, n: u32) -> *mut c_char;
    fn tls_id_on_xmppaddr_num(conn: *mut ffi::xmpp_conn_t) -> u32;
}

unsafe fn show_tls_info(conn: *mut ffi::xmpp_conn_t) {
    println!("\n--- TLS Information ---");

    // Show XMPP addresses from certificate
    let num_addrs = tls_id_on_xmppaddr_num(conn);
    println!("XMPP addresses in certificate: {}", num_addrs);

    for i in 0..num_addrs {
        let addr_ptr = tls_id_on_xmppaddr(conn, i);
        if !addr_ptr.is_null() {
            let addr_cstr = CStr::from_ptr(addr_ptr);
            if let Ok(addr) = addr_cstr.to_str() {
                println!("  [{}] {}", i, addr);
            }
            // Free the returned string
            libc::free(addr_ptr as *mut c_void);
        }
    }

    println!("----------------------\n");
}
