use native_tls::{TlsConnector, TlsStream};
use std::io::{self, Read, Write};
use std::net::TcpStream;

use eeeemail_domain::{MailError, MailErrorCode, Result};

/// A socket that may or may not be TLS wrapped.
///
/// IMAP needs three connection modes (implicit TLS on 993, STARTTLS on 143, and
/// plaintext for local relays). Boxing a `dyn Read + Write` would need a second
/// allocation on every byte path, so a small enum keeps `imap::Session<T>` over
/// one concrete type.
pub enum MaybeTlsStream {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl Read for MaybeTlsStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            MaybeTlsStream::Plain(s) => s.read(buf),
            MaybeTlsStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for MaybeTlsStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            MaybeTlsStream::Plain(s) => s.write(buf),
            MaybeTlsStream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            MaybeTlsStream::Plain(s) => s.flush(),
            MaybeTlsStream::Tls(s) => s.flush(),
        }
    }
}

/// Read one CRLF-terminated IMAP line.
///
/// Only used for the pre-login handshake (greeting + STARTTLS), where we cannot
/// go through `imap::Client` yet.
pub fn read_line<R: Read>(stream: &mut R) -> io::Result<String> {
    let mut out = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte)?;
        if n == 0 {
            break;
        }
        out.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&out).trim_end().to_string())
}

/// Read the server greeting (`* OK …`).
pub fn expect_greeting<R: Read>(stream: &mut R) -> Result<String> {
    let line = read_line(stream).map_err(|e| {
        MailError::new(
            MailErrorCode::NetworkUnavailable,
            format!("failed to read IMAP greeting: {e}"),
        )
    })?;
    if line.starts_with("* BYE") || line.is_empty() {
        return Err(MailError::new(
            MailErrorCode::ProviderUnavailable,
            format!("server refused the connection: {line}"),
        ));
    }
    Ok(line)
}

/// Issue the STARTTLS upgrade and wrap the socket, consuming the plaintext
/// stream and returning the TLS stream.
pub fn starttls(stream: TcpStream, host: &str) -> Result<MaybeTlsStream> {
    let mut stream = stream;
    stream
        .write_all(b"A1 STARTTLS\r\n")
        .and_then(|_| stream.flush())
        .map_err(|e| {
            MailError::new(
                MailErrorCode::NetworkUnavailable,
                format!("STARTTLS failed: {e}"),
            )
        })?;

    loop {
        let line = read_line(&mut stream).map_err(|e| {
            MailError::new(
                MailErrorCode::NetworkUnavailable,
                format!("STARTTLS failed: {e}"),
            )
        })?;
        if line.is_empty() {
            return Err(MailError::new(
                MailErrorCode::ProviderUnavailable,
                "server closed the connection during STARTTLS",
            ));
        }
        if line.starts_with("A1 ") {
            if !line[3..].to_ascii_uppercase().starts_with("OK") {
                return Err(MailError::new(
                    MailErrorCode::ProviderUnavailable,
                    format!("STARTTLS rejected: {line}"),
                ));
            }
            break;
        }
    }

    let connector = TlsConnector::builder().build().map_err(|e| {
        MailError::new(
            MailErrorCode::NetworkUnavailable,
            format!("cannot build TLS connector: {e}"),
        )
    })?;
    let tls = connector.connect(host, stream).map_err(|e| {
        MailError::new(
            MailErrorCode::NetworkUnavailable,
            format!("TLS handshake with {host} failed: {e}"),
        )
    })?;
    Ok(MaybeTlsStream::Tls(tls))
}

/// Wrap an existing plaintext socket in implicit TLS.
pub fn wrap_tls(stream: TcpStream, host: &str) -> Result<MaybeTlsStream> {
    let connector = TlsConnector::builder().build().map_err(|e| {
        MailError::new(
            MailErrorCode::NetworkUnavailable,
            format!("cannot build TLS connector: {e}"),
        )
    })?;
    connector
        .connect(host, stream)
        .map(MaybeTlsStream::Tls)
        .map_err(|e| {
            MailError::new(
                MailErrorCode::NetworkUnavailable,
                format!("TLS handshake with {host} failed: {e}"),
            )
        })
}
