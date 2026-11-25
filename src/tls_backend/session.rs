//! TLS session implementation using rustls
//!
//! TlsSession wraps rustls::ClientConnection and handles I/O on a raw socket fd.
//! Socket ownership stays with libstrophe - we only read/write, never close.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::ffi::CString;
use libc::{c_int, c_void, size_t, ssize_t, EAGAIN};
use rustls::{ClientConfig, ClientConnection, RootCertStore};
use rustls::pki_types::{ServerName, CertificateDer};
use sha2::{Sha256, Digest};
use super::cert;

/// Wrapper around raw socket fd implementing Read/Write for rustls::Stream
///
/// ponytail: Never closes the fd - libstrophe owns it
struct RawSock {
    fd: c_int,
}

impl RawSock {
    fn new(fd: c_int) -> Self {
        Self { fd }
    }
}

impl Read for RawSock {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let ret = unsafe {
            libc::read(
                self.fd,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as size_t,
            )
        };

        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret as usize)
        }
    }
}

impl Write for RawSock {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let ret = unsafe {
            libc::write(
                self.fd,
                buf.as_ptr() as *const c_void,
                buf.len() as size_t,
            )
        };

        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret as usize)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        // Raw sockets don't buffer
        Ok(())
    }
}

/// TLS session wrapping rustls::ClientConnection
///
/// ponytail: Stores socket fd by value, never closes it
pub struct TlsSession {
    conn: ClientConnection,
    sock: c_int,
    last_errno: i32,
    peer_certs: Vec<CertificateDer<'static>>,
    xmpp_addrs: Vec<CString>,
    channel_binding: Option<Vec<u8>>,
}

impl TlsSession {
    /// Create new TLS session for given socket and domain
    ///
    /// Uses webpki-roots for certificate validation
    pub fn new(sock: c_int, domain: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Build root cert store with Mozilla roots
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        let server_name = ServerName::try_from(domain.to_string())?;

        let conn = ClientConnection::new(Arc::new(config), server_name)?;

        Ok(Self {
            conn,
            sock,
            last_errno: 0,
            peer_certs: Vec::new(),
            xmpp_addrs: Vec::new(),
            channel_binding: None,
        })
    }

    /// Perform TLS handshake
    ///
    /// Returns 0 on success, -1 on error
    pub fn start(&mut self) -> c_int {
        let mut raw_sock = RawSock::new(self.sock);

        // Drive handshake until complete or would block
        loop {
            if !self.conn.is_handshaking() {
                // Handshake complete - save peer certificates and extract XMPP addrs
                if let Some(certs) = self.conn.peer_certificates() {
                    self.peer_certs = certs.iter().cloned().collect();
                    if !self.peer_certs.is_empty() {
                        if let Ok(addrs) = cert::extract_xmpp_addrs(&self.peer_certs[0]) {
                            self.xmpp_addrs = cert::strings_to_c(addrs);
                        }
                    }
                }
                return 0;
            }

            match self.conn.complete_io(&mut raw_sock) {
                Ok(_) => {
                    if !self.conn.is_handshaking() {
                        // Handshake complete - save peer certificates and extract XMPP addrs
                        if let Some(certs) = self.conn.peer_certificates() {
                            self.peer_certs = certs.iter().cloned().collect();
                            if !self.peer_certs.is_empty() {
                                if let Ok(addrs) = cert::extract_xmpp_addrs(&self.peer_certs[0]) {
                                    self.xmpp_addrs = cert::strings_to_c(addrs);
                                }
                            }
                        }
                        return 0;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.last_errno = EAGAIN;
                    return 0; // Need more data, return success (non-blocking)
                }
                Err(e) => {
                    self.last_errno = e.raw_os_error().unwrap_or(libc::EIO);
                    return -1;
                }
            }
        }
    }

    /// Read decrypted data from TLS session
    ///
    /// Returns number of bytes read, 0 on EAGAIN, -1 on error
    pub fn read(&mut self, buf: &mut [u8]) -> ssize_t {
        let mut raw_sock = RawSock::new(self.sock);

        // Process any pending TLS data first
        match self.conn.complete_io(&mut raw_sock) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.last_errno = EAGAIN;
                return 0;
            }
            Err(e) => {
                self.last_errno = e.raw_os_error().unwrap_or(libc::EIO);
                return -1;
            }
        }

        // Read decrypted application data
        match self.conn.reader().read(buf) {
            Ok(n) => n as ssize_t,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.last_errno = EAGAIN;
                0
            }
            Err(e) => {
                self.last_errno = e.raw_os_error().unwrap_or(libc::EIO);
                -1
            }
        }
    }

    /// Write encrypted data to TLS session
    ///
    /// Returns number of bytes written, 0 on EAGAIN, -1 on error
    pub fn write(&mut self, buf: &[u8]) -> ssize_t {
        // Write plaintext to TLS session
        match self.conn.writer().write(buf) {
            Ok(n) => {
                // Flush encrypted data to socket
                let mut raw_sock = RawSock::new(self.sock);
                match self.conn.complete_io(&mut raw_sock) {
                    Ok(_) => n as ssize_t,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        self.last_errno = EAGAIN;
                        0
                    }
                    Err(e) => {
                        self.last_errno = e.raw_os_error().unwrap_or(libc::EIO);
                        -1
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.last_errno = EAGAIN;
                0
            }
            Err(e) => {
                self.last_errno = e.raw_os_error().unwrap_or(libc::EIO);
                -1
            }
        }
    }

    /// Check if there's pending data to read
    pub fn pending(&self) -> bool {
        self.conn.wants_read()
    }

    /// Get last errno
    pub fn last_error(&self) -> i32 {
        self.last_errno
    }

    /// Get peer certificates (empty if handshake not complete)
    pub fn peer_certificates(&self) -> &[CertificateDer<'static>] {
        &self.peer_certs
    }

    /// Get number of XMPP addresses from peer cert
    pub fn xmpp_addr_count(&self) -> u32 {
        self.xmpp_addrs.len() as u32
    }

    /// Get XMPP address at index (returns None if out of bounds)
    pub fn xmpp_addr(&self, n: u32) -> Option<&CString> {
        self.xmpp_addrs.get(n as usize)
    }

    /// Initialize channel binding (tls-server-end-point)
    ///
    /// Returns 0 on success, -1 on error
    pub fn init_channel_binding(&mut self) -> c_int {
        // Compute SHA-256 hash of server certificate
        if self.peer_certs.is_empty() {
            return -1;
        }

        let mut hasher = Sha256::new();
        hasher.update(&self.peer_certs[0]);
        let hash = hasher.finalize();

        self.channel_binding = Some(hash.to_vec());
        0
    }

    /// Get channel binding data
    ///
    /// Returns (prefix, data) or None if not initialized
    pub fn channel_binding_data(&self) -> Option<(&'static str, &[u8])> {
        self.channel_binding
            .as_ref()
            .map(|data| ("tls-server-end-point", data.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::RawFd;

    #[test]
    fn test_raw_sock_io() {
        // Arrange: Create socketpair
        let mut fds = [0 as RawFd, 0 as RawFd];
        let ret = unsafe {
            libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr())
        };
        assert_eq!(ret, 0, "socketpair failed");

        let mut sock_a = RawSock::new(fds[0]);
        let mut sock_b = RawSock::new(fds[1]);

        // Act: Write from A, read from B
        let data = b"hello world";
        let written = sock_a.write(data).expect("write failed");
        assert_eq!(written, data.len());

        let mut buf = [0u8; 128];
        let read = sock_b.read(&mut buf).expect("read failed");

        // Assert: Data matches
        assert_eq!(read, data.len());
        assert_eq!(&buf[..read], data);

        // Cleanup
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn test_channel_binding_uninitialized() {
        // Session without handshake has no binding
        let session = TlsSession::new(-1, "test.example.com").unwrap();
        assert!(session.channel_binding_data().is_none());
    }
}
