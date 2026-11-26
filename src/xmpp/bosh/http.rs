//! Minimal HTTP/1.1 POST client for BOSH.
//!
//! shortcut: no redirects (3xx is returned as a status), no proxies, no cookies,
//! no compression (`Accept-Encoding: identity`); add them when a deployment needs one.

use crate::xmpp::error::{Error, Result};
use crate::xmpp::net::{self, Endpoint, Transport};
use futures_util::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use std::time::Duration;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// One keep-alive HTTP connection to the BOSH connection manager.
pub(crate) struct HttpConn {
    ep: Endpoint,
    io: Option<BufReader<Transport>>,
}

impl HttpConn {
    pub(crate) fn new(ep: Endpoint) -> Self {
        Self { ep, io: None }
    }

    /// POST `body`; returns `(status, response body)`. Fails with [`Error::Io`]
    /// if no full response arrives within `limit`.
    pub(crate) async fn post(&mut self, body: &str, limit: Duration) -> Result<(u16, String)> {
        let r = net::timeout(limit, async {
            let reused = self.io.is_some();
            match self.exchange(body).await {
                // A reused keep-alive connection may have been closed by the server; retry once fresh.
                Err(Error::Io) if reused => {
                    self.io = None;
                    self.exchange(body).await
                }
                r => r,
            }
        })
        .await;
        if r.is_err() {
            self.io = None;
        }
        r
    }

    async fn exchange(&mut self, body: &str) -> Result<(u16, String)> {
        if self.io.is_none() {
            self.io = Some(BufReader::new(net::connect(&self.ep).await?));
        }
        let io = self.io.as_mut().unwrap();
        let req = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\nAccept-Encoding: identity\r\nConnection: keep-alive\r\n\r\n{}",
            self.ep.path,
            self.ep.host_header(),
            body.len(),
            body
        );
        io.write_all(req.as_bytes()).await.map_err(|_| Error::Io)?;
        io.flush().await.map_err(|_| Error::Io)?;
        let (status, resp, keep_alive) = read_response(io).await?;
        if !keep_alive {
            self.io = None;
        }
        Ok((status, resp))
    }
}

/// Parse one HTTP/1.1 response: `(status, body, keep_alive)`.
pub(crate) async fn read_response<R: AsyncBufRead + Unpin>(r: &mut R) -> Result<(u16, String, bool)> {
    let mut budget = MAX_HEADER_BYTES;
    let status_line = read_line(r, &mut budget).await?;
    if status_line.is_empty() {
        return Err(Error::Io); // EOF before any response: connection closed
    }
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(Error::Protocol);
    }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or(Error::Protocol)?;
    let mut keep_alive = version == "HTTP/1.1";
    let mut length: Option<usize> = None;
    let mut chunked = false;
    loop {
        let line = read_line(r, &mut budget).await?;
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(Error::Protocol);
        };
        let value = value.trim().to_ascii_lowercase();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = Some(value.parse().map_err(|_| Error::Protocol)?),
            "transfer-encoding" => chunked = value.split(',').any(|v| v.trim() == "chunked"),
            "connection" if value.contains("close") => keep_alive = false,
            "connection" if value.contains("keep-alive") => keep_alive = true,
            _ => {}
        }
    }
    let body = if chunked {
        read_chunked(r, &mut budget).await?
    } else if let Some(n) = length {
        if n > MAX_BODY_BYTES {
            return Err(Error::Protocol);
        }
        let mut buf = vec![0; n];
        r.read_exact(&mut buf).await.map_err(|_| Error::Io)?;
        buf
    } else {
        keep_alive = false; // body ends at EOF
        let mut buf = Vec::new();
        r.take(MAX_BODY_BYTES as u64 + 1).read_to_end(&mut buf).await.map_err(|_| Error::Io)?;
        if buf.len() > MAX_BODY_BYTES {
            return Err(Error::Protocol);
        }
        buf
    };
    let body = String::from_utf8(body).map_err(|_| Error::Protocol)?;
    Ok((status, body, keep_alive))
}

async fn read_chunked<R: AsyncBufRead + Unpin>(r: &mut R, budget: &mut usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line = read_line(r, budget).await?;
        let size_hex = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| Error::Protocol)?;
        if size == 0 {
            while !read_line(r, budget).await?.is_empty() {} // trailers
            return Ok(body);
        }
        if body.len() + size > MAX_BODY_BYTES {
            return Err(Error::Protocol);
        }
        let start = body.len();
        body.resize(start + size, 0);
        r.read_exact(&mut body[start..]).await.map_err(|_| Error::Io)?;
        if !read_line(r, budget).await?.is_empty() {
            return Err(Error::Protocol); // chunk data must end with CRLF
        }
    }
}

/// Read one CRLF/LF-terminated line without the terminator, charging `budget`
/// (header bytes, so a hostile server cannot grow memory without bound).
/// Returns `""` both for an empty line and at EOF; callers that care check first-line EOF.
async fn read_line<R: AsyncBufRead + Unpin>(r: &mut R, budget: &mut usize) -> Result<String> {
    let mut buf = Vec::new();
    let n = (&mut *r)
        .take(*budget as u64)
        .read_until(b'\n', &mut buf)
        .await
        .map_err(|_| Error::Io)?;
    if n > 0 && !buf.ends_with(b"\n") {
        return Err(if n == *budget { Error::Protocol } else { Error::Io });
    }
    *budget -= n;
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    String::from_utf8(buf).map_err(|_| Error::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::io::Cursor;

    fn parse(raw: &str) -> Result<(u16, String, bool)> {
        smol::block_on(read_response(&mut Cursor::new(raw.as_bytes().to_vec())))
    }

    #[test]
    fn content_length_body() {
        let r = parse("HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: 7\r\n\r\n<body/>").unwrap();
        assert_eq!(r, (200, "<body/>".into(), true));
    }

    #[test]
    fn chunked_body_with_extension_and_trailer() {
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3;x=y\r\n<bo\r\n4\r\ndy/>\r\n0\r\nX-T: 1\r\n\r\n";
        assert_eq!(parse(raw).unwrap(), (200, "<body/>".into(), true));
    }

    #[test]
    fn connection_close_and_http10() {
        let r = parse("HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").unwrap();
        assert!(!r.2);
        let r = parse("HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n").unwrap();
        assert_eq!((r.0, r.2), (404, false));
        let r = parse("HTTP/1.0 200 OK\r\nConnection: keep-alive\r\nContent-Length: 0\r\n\r\n").unwrap();
        assert!(r.2);
    }

    #[test]
    fn body_until_eof_without_length() {
        let r = parse("HTTP/1.1 200 OK\r\n\r\n<body/>").unwrap();
        assert_eq!(r, (200, "<body/>".into(), false));
    }

    #[test]
    fn eof_and_truncation_are_io_errors() {
        assert_eq!(parse(""), Err(Error::Io));
        assert_eq!(parse("HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n<body/>"), Err(Error::Io));
    }

    #[test]
    fn malformed_and_oversized_are_protocol_errors() {
        assert_eq!(parse("SMTP 220 hi\r\n\r\n"), Err(Error::Protocol));
        assert_eq!(parse("HTTP/1.1 abc OK\r\n\r\n"), Err(Error::Protocol));
        assert_eq!(parse("HTTP/1.1 200 OK\r\nbad header\r\n\r\n"), Err(Error::Protocol));
        let huge = format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEADER_BYTES));
        assert_eq!(parse(&huge), Err(Error::Protocol));
        let big = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", MAX_BODY_BYTES + 1);
        assert_eq!(parse(&big), Err(Error::Protocol));
    }
}
