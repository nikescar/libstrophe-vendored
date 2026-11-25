//! rustls+ring TLS backend for libstrophe
//!
//! Provides C-compatible TLS functions using pure Rust TLS implementation.

#[cfg(feature = "tls-rustls")]
pub mod session;

#[cfg(feature = "tls-rustls")]
pub mod cert;

#[cfg(feature = "tls-rustls")]
pub mod ffi;
