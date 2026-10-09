//! Loads certificates and opens encrypted connections with a fixed deadline.

use clap::Args;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Maximum socket I/O time for the handshake, command, and reply together.
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum message size, including its final newline.
pub const MAX_MESSAGE_BYTES: u64 = 16 * 1024;

/// Paths to trusted CA certificates, our certificate, and our private key.
#[derive(Args, Debug)]
pub struct TlsFiles {
    /// CA certificates trusted to sign the other side's identity.
    #[arg(long)]
    pub ca_cert: PathBuf,
    /// Our certificate chain in PEM format.
    #[arg(long)]
    pub cert: PathBuf,
    /// Our private key in PEM format. Keep this file private.
    #[arg(long)]
    pub private_key: PathBuf,
}

/// Loads certificates, returning an error for unreadable or empty files.
/// Loading does not establish trust; TLS verifies the peer later.
pub fn load_certificates(path: &Path) -> io::Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let certificates = rustls_pemfile::certs(&mut reader).collect::<io::Result<Vec<_>>>()?;

    if certificates.is_empty() {
        return Err(io::Error::other("No certificates found"));
    }

    Ok(certificates)
}

/// Loads the first private key, rejecting missing keys or broad Unix permissions.
pub fn load_private_key(path: &Path) -> io::Result<PrivateKeyDer<'static>> {
    let file = File::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::other(
                "Private key must have owner-only permissions; use chmod 600",
            ));
        }
    }
    let mut reader = BufReader::new(file);

    let key = rustls_pemfile::private_key(&mut reader)?;

    match key {
        Some(key) => Ok(key),
        None => Err(io::Error::other("No private key found")),
    }
}

/// Loads trusted CA certificates, rejecting invalid certificates.
pub fn load_roots(path: &Path) -> io::Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    for certificate in load_certificates(path)? {
        roots.add(certificate).map_err(io::Error::other)?;
    }
    Ok(roots)
}

/// Requires a trusted client certificate and supplies the agent's identity.
/// Every accepted client still needs the shared token to run commands.
pub fn server_config(files: &TlsFiles) -> io::Result<ServerConfig> {
    let roots = load_roots(&files.ca_cert)?;
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(io::Error::other)?;

    ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            load_certificates(&files.cert)?,
            load_private_key(&files.private_key)?,
        )
        .map_err(io::Error::other)
}

/// Verifies the agent's certificate and supplies the client's identity.
pub fn client_config(files: &TlsFiles) -> io::Result<ClientConfig> {
    ClientConfig::builder()
        .with_root_certificates(load_roots(&files.ca_cert)?)
        .with_client_auth_cert(
            load_certificates(&files.cert)?,
            load_private_key(&files.private_key)?,
        )
        .map_err(io::Error::other)
}

/// An agent-side TLS connection with bounded socket I/O.
pub type ServerStream = StreamOwned<ServerConnection, DeadlineStream>;
/// A client-side TLS connection with bounded socket I/O.
pub type ClientStream = StreamOwned<ClientConnection, DeadlineStream>;

/// Completes client authentication before any command is read.
pub fn accept(socket: TcpStream, config: Arc<ServerConfig>) -> io::Result<ServerStream> {
    socket.set_nonblocking(false)?;
    let connection = ServerConnection::new(config).map_err(io::Error::other)?;
    let socket = DeadlineStream::new(socket, Instant::now() + CONNECTION_TIMEOUT);
    let mut stream = StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    Ok(stream)
}

/// Connects to a numeric IP and verifies the expected server name.
/// Uses an already-loaded config and opens a new connection with a fixed deadline.
///
/// # Errors
/// Returns an error if the address or server name is invalid, connecting fails,
/// or the TLS handshake fails.
pub fn connect(
    address: &str,
    server_name: &str,
    config: Arc<ClientConfig>,
) -> io::Result<ClientStream> {
    let name = ServerName::try_from(server_name.to_string()).map_err(io::Error::other)?;
    let connection = ClientConnection::new(config, name).map_err(io::Error::other)?;
    let address: SocketAddr = address.parse().map_err(io::Error::other)?;
    let deadline = Instant::now() + CONNECTION_TIMEOUT;
    let socket = TcpStream::connect_timeout(&address, CONNECTION_TIMEOUT)?;
    let socket = DeadlineStream::new(socket, deadline);
    let mut stream = StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    Ok(stream)
}

/// Reads one newline-terminated message without allowing unlimited allocation.
/// Returns an error for oversized, incomplete, or invalid UTF-8 messages.
pub fn read_message(stream: &mut impl Read) -> io::Result<String> {
    let mut input = String::new();
    BufReader::new(stream.take(MAX_MESSAGE_BYTES + 1)).read_line(&mut input)?;
    if input.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(io::Error::other("Message exceeds 16 KiB"));
    }
    if !input.ends_with('\n') {
        return Err(io::Error::other("Message must end with a newline"));
    }
    Ok(input)
}

/// Limits socket I/O to one deadline across the handshake, command, and reply.
pub struct DeadlineStream {
    socket: TcpStream,
    deadline: Instant,
}

impl DeadlineStream {
    fn new(socket: TcpStream, deadline: Instant) -> Self {
        Self { socket, deadline }
    }

    fn remaining(&self) -> io::Result<Duration> {
        match self.deadline.checked_duration_since(Instant::now()) {
            Some(time) if !time.is_zero() => Ok(time),
            _ => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Connection deadline exceeded",
            )),
        }
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.socket.set_read_timeout(Some(self.remaining()?))?;
        self.socket.read(bytes)
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.socket.set_write_timeout(Some(self.remaining()?))?;
        self.socket.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.remaining()?;
        self.socket.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_one_complete_message() {
        let mut input = Cursor::new(b"{\"command\":\"status\"}\n");
        assert_eq!(
            read_message(&mut input).unwrap(),
            "{\"command\":\"status\"}\n"
        );
    }

    #[test]
    fn rejects_incomplete_messages() {
        assert!(read_message(&mut Cursor::new(b"{}")).is_err());
        assert!(read_message(&mut Cursor::new(b"")).is_err());
    }

    #[test]
    fn rejects_oversized_messages() {
        let mut bytes = vec![b'x'; MAX_MESSAGE_BYTES as usize];
        bytes.push(b'\n');
        assert!(read_message(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn accepts_message_at_limit() {
        let mut bytes = vec![b'x'; MAX_MESSAGE_BYTES as usize - 1];
        bytes.push(b'\n');
        assert!(read_message(&mut Cursor::new(bytes)).is_ok());
    }

    #[test]
    fn expired_deadline_rejects_io() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut stream = DeadlineStream::new(socket, Instant::now() - Duration::from_secs(1));
        assert_eq!(
            stream.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(
            stream.write(b"x").unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
}
