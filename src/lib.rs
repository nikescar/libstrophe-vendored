//! # libstrophe-vendored
//!
//! Vendored libstrophe C library with Rust FFI bindings and rustls TLS backend.
//!
//! This crate provides:
//! - Raw FFI bindings to libstrophe C API
//! - rustls-based TLS implementation (no OpenSSL dependency)
//! - Vendored C sources (libstrophe + expat)
//!
//! ## License
//!
//! This crate is MIT licensed. It vendors:
//! - libstrophe (MIT OR GPL-3.0, using MIT)
//! - expat (MIT)
//!
//! ## Usage
//!
//! This crate provides low-level FFI bindings. Use the `ffi` module to access
//! the C API directly:
//!
//! ```rust,no_run
//! use libstrophe_vendored::ffi;
//! use std::ptr;
//!
//! unsafe {
//!     ffi::xmpp_initialize();
//!     let ctx = ffi::xmpp_ctx_new(ptr::null_mut(), ptr::null_mut());
//!     let conn = ffi::xmpp_conn_new(ctx);
//!     // ... use libstrophe API
//!     ffi::xmpp_conn_release(conn);
//!     ffi::xmpp_ctx_free(ctx);
//!     ffi::xmpp_shutdown();
//! }
//! ```
//!
//! See libstrophe documentation: <https://strophe.im/libstrophe/>

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::all)]

/// Raw FFI bindings to libstrophe C API
///
/// Auto-generated bindings to the libstrophe C library.
/// See: <https://strophe.im/libstrophe/doc/0.14.0/>
pub mod ffi;

/// rustls-based TLS backend for libstrophe
///
/// Provides TLS implementation using rustls instead of OpenSSL.
#[cfg(feature = "tls-rustls")]
pub mod tls_backend;
