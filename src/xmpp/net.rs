//! Shared async TCP/TLS transport for the WebSocket and BOSH clients.
//!
//! TLS uses rustls with the ring provider and the Mozilla root store
//! (`webpki-roots`), the same trust anchors as the libstrophe TLS backend.

use super::error::{Error, Result};
use futures_rustls::client::TlsStream;
use futures_rustls::rustls::{self, pki_types::ServerName, ClientConfig, RootCertStore};
use futures_rustls::TlsConnector;
use futures_util::io::{AsyncRead, AsyncWrite};
use smol::net::TcpStream;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Parsed `ws://`, `wss://`, `http://` or `https://` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub tls: bool,
    /// Host without IPv6 brackets.
    pub host: String,
    pub port: u16,
    /// Path plus query, always starting with `/`.
    pub path: String,
}

impl Endpoint {
    /// `Host` header value (re-adds IPv6 brackets, omits default port).
    pub(crate) fn host_header(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == if self.tls { 443 } else { 80 } {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }
}

pub(crate) fn parse_url(url: &str) -> Result<Endpoint> {
    let (scheme, rest) = url.split_once("://").ok_or(Error::Protocol)?;
    let tls = match scheme.to_ascii_lowercase().as_str() {
        "ws" | "http" => false,
        "wss" | "https" => true,
        _ => return Err(Error::Protocol),
    };
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_owned()),
        Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
        None => (rest, "/".to_owned()),
    };
    if authority.contains('@') {
        return Err(Error::Protocol); // no userinfo: credentials go through SASL
    }
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, tail) = v6.split_once(']').ok_or(Error::Protocol)?;
        (h, tail.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Err(Error::Protocol);
    }
    let port = match port {
        Some(p) => p.parse::<u16>().map_err(|_| Error::Protocol)?,
        None if tls => 443,
        None => 80,
    };
    Ok(Endpoint { tls, host: host.to_owned(), port, path })
}

/// Plain TCP or rustls-over-TCP stream.
pub(crate) enum Transport {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for Transport {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Transport::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Transport::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Transport {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Transport::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Transport::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Transport::Plain(s) => Pin::new(s).poll_flush(cx),
            Transport::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Transport::Plain(s) => Pin::new(s).poll_close(cx),
            Transport::Tls(s) => Pin::new(s.as_mut()).poll_close(cx),
        }
    }
}

/// Client config with an explicit ring provider, so a second rustls provider
/// elsewhere in the dependency graph cannot make the builder panic.
pub(crate) fn tls_config() -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Tls)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Open TCP to `ep`, then run a verified TLS handshake when `ep.tls`.
pub(crate) async fn connect(ep: &Endpoint) -> Result<Transport> {
    let tcp = timeout(CONNECT_TIMEOUT, async {
        TcpStream::connect((ep.host.as_str(), ep.port)).await.map_err(|_| Error::Io)
    })
    .await?;
    if !ep.tls {
        return Ok(Transport::Plain(tcp));
    }
    let name = ServerName::try_from(ep.host.clone()).map_err(|_| Error::Tls)?;
    let tls = timeout(CONNECT_TIMEOUT, async {
        TlsConnector::from(tls_config()?).connect(name, tcp).await.map_err(|_| Error::Tls)
    })
    .await?;
    Ok(Transport::Tls(Box::new(tls)))
}

/// Fail with [`Error::Io`] if `f` does not finish within `d`.
pub(crate) async fn timeout<T>(d: Duration, f: impl Future<Output = Result<T>>) -> Result<T> {
    smol::future::or(f, async {
        smol::Timer::after(d).await;
        Err(Error::Io)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(tls: bool, host: &str, port: u16, path: &str) -> Endpoint {
        Endpoint { tls, host: host.into(), port, path: path.into() }
    }

    #[test]
    fn parse_schemes_and_default_ports() {
        assert_eq!(parse_url("ws://h/x").unwrap(), ep(false, "h", 80, "/x"));
        assert_eq!(parse_url("wss://h/x").unwrap(), ep(true, "h", 443, "/x"));
        assert_eq!(parse_url("http://h:5280/http-bind").unwrap(), ep(false, "h", 5280, "/http-bind"));
        assert_eq!(parse_url("HTTPS://h").unwrap(), ep(true, "h", 443, "/"));
        assert_eq!(parse_url("https://h?a=1").unwrap(), ep(true, "h", 443, "/?a=1"));
    }

    #[test]
    fn parse_ipv6() {
        assert_eq!(parse_url("wss://[::1]:5443/ws").unwrap(), ep(true, "::1", 5443, "/ws"));
        assert_eq!(parse_url("https://[::1]/").unwrap(), ep(true, "::1", 443, "/"));
        assert_eq!(parse_url("https://[::1]:8443/").unwrap().host_header(), "[::1]:8443");
    }

    #[test]
    fn parse_rejects_bad_urls() {
        for bad in ["ftp://h/", "h/x", "wss://", "wss://:5/", "https://u:p@h/", "wss://h:99999/", "wss://[::1/"] {
            assert_eq!(parse_url(bad), Err(Error::Protocol), "{bad}");
        }
    }

    #[test]
    fn host_header_omits_default_port() {
        assert_eq!(ep(true, "h", 443, "/").host_header(), "h");
        assert_eq!(ep(false, "h", 5280, "/").host_header(), "h:5280");
    }

    #[test]
    fn tls_config_builds_without_provider_panic() {
        assert!(tls_config().is_ok());
    }

    /// A `wss://`/`https://` endpoint must start with a TLS ClientHello, never plaintext.
    #[test]
    fn tls_endpoint_sends_client_hello_and_rejects_non_tls_peer() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut first = [0u8; 1];
            s.read_exact(&mut first).unwrap();
            first[0] // dropping `s` closes the connection mid-handshake
        });
        // `localhost` because rustls ServerName for IP needs an IP-SAN cert either way.
        let r = smol::block_on(connect(&ep(true, "localhost", port, "/")));
        assert!(matches!(r, Err(Error::Tls)), "handshake must fail with Error::Tls");
        assert_eq!(server.join().unwrap(), 0x16, "first byte must be a TLS handshake record");
    }
}
