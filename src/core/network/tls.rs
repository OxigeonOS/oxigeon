//! TLS for the listeners that want it — `wss://` and `telnets://`.
//!
//! One acceptor type serves both, because by the time a stream reaches either
//! transport the handshake is over and what is left is an ordinary
//! `AsyncRead + AsyncWrite`. That is the whole reason this is a module rather
//! than two: TLS is a wrapper around the socket, not a property of the protocol
//! above it.
//!
//! There is no client-certificate support and no ACME, but a certificate *is*
//! re-read when it changes on disk, so `certbot` renewal does not need a
//! restart — see [`ReloadingCert`].

use std::io;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;

use crate::core::lock::RwLockExt;
use crate::error::{OxigeonError, Result};

/// A connection that may or may not be encrypted.
///
/// An enum rather than a `Box<dyn>` or a generic parameter. The `dyn` version
/// costs a virtual call on every read and write of a hot path; the generic
/// version spreads a type parameter through `TelnetConnection`, its listener,
/// the relay and every signature in between, to express a choice that is made
/// once at accept time and never again.
///
/// Both variants are `Unpin`, which is what lets the projections below be
/// `Pin::new` rather than `unsafe`.
pub enum MaybeTls {
    Plain(TcpStream),
    /// Boxed: `ServerConnection` is a large struct and this enum is moved
    /// around by value at accept time. Without the box every plaintext
    /// connection pays its size too.
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

impl MaybeTls {
    /// Whether this connection is encrypted, for logging and `session_info`.
    pub fn is_tls(&self) -> bool {
        matches!(self, MaybeTls::Tls(_))
    }
}

impl AsyncRead for MaybeTls {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTls {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_write(cx, buf),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_flush(cx),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MaybeTls::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MaybeTls::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// The certificate a listener is currently serving, and where it came from.
///
/// A resolver rather than a fixed certificate so the files can be re-read while
/// the server runs. `certbot` renews on its own schedule and restarting a MUD to
/// pick up a new certificate means disconnecting everyone on it — which is the
/// kind of cost that gets renewal quietly disabled instead.
///
/// Only *new* handshakes see a reloaded certificate. A TLS session already
/// established keeps the one it negotiated with, which is correct: the
/// certificate authenticated that connection when it opened.
#[derive(Debug)]
pub struct ReloadingCert {
    cert_path: String,
    key_path: String,
    current: std::sync::RwLock<Arc<rustls::sign::CertifiedKey>>,
    /// What the files looked like when they were last read. Modification time
    /// and length together, because a renewal can land inside a filesystem's
    /// timestamp granularity and a length change is the cheap tiebreak.
    seen: std::sync::RwLock<Stamp>,
}

type Stamp = ((Option<std::time::SystemTime>, u64), (Option<std::time::SystemTime>, u64));

fn stamp_of(path: &str) -> (Option<std::time::SystemTime>, u64) {
    match std::fs::metadata(path) {
        Ok(m) => (m.modified().ok(), m.len()),
        Err(_) => (None, 0),
    }
}

impl ReloadingCert {
    fn load(cert_path: &str, key_path: &str) -> Result<Arc<rustls::sign::CertifiedKey>> {
        let certs = load_certs(cert_path)?;
        let key = load_key(key_path)?;
        // `from_der` checks the key against the certificate's public key, so a
        // mismatched pair is caught here rather than at the first handshake.
        rustls::sign::CertifiedKey::from_der(certs, key, &rustls::crypto::ring::default_provider())
            .map(Arc::new)
            .map_err(|e| {
                OxigeonError::Config(format!(
                    "TLS: certificate {cert_path} and key {key_path} do not go together: {e}"
                ))
            })
    }

    fn new(cert_path: &str, key_path: &str) -> Result<Self> {
        let current = Self::load(cert_path, key_path)?;
        Ok(ReloadingCert {
            current: std::sync::RwLock::new(current),
            seen: std::sync::RwLock::new((stamp_of(cert_path), stamp_of(key_path))),
            cert_path: cert_path.to_string(),
            key_path: key_path.to_string(),
        })
    }

    /// Re-read the files if either has changed. Returns whether it swapped.
    ///
    /// **A failed reload keeps the old certificate**, loudly. Renewal is not
    /// atomic — there is a moment where the certificate is the new one and the
    /// key is still the old — so a poll that lands mid-write must not take the
    /// listener down. The next tick picks it up.
    pub fn reload_if_changed(&self) -> bool {
        let now = (stamp_of(&self.cert_path), stamp_of(&self.key_path));
        if *self.seen.read_recover() == now {
            return false;
        }

        match Self::load(&self.cert_path, &self.key_path) {
            Ok(fresh) => {
                *self.current.write_recover() = fresh;
                *self.seen.write_recover() = now;
                tracing::info!(
                    "TLS: reloaded certificate {} and key {}",
                    self.cert_path,
                    self.key_path
                );
                true
            }
            Err(e) => {
                // Deliberately not recording the new stamp: leaving it unchanged
                // means the next tick tries again, which is what makes a
                // half-written pair recover on its own.
                tracing::warn!(
                    "TLS: {} or {} changed but would not load, keeping the previous \
                     certificate: {}",
                    self.cert_path,
                    self.key_path,
                    e
                );
                false
            }
        }
    }
}

impl rustls::server::ResolvesServerCert for ReloadingCert {
    fn resolve(
        &self,
        _hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.current.read_recover().clone())
    }
}

/// Build an acceptor from a PEM certificate chain and its private key.
///
/// With `reload_seconds > 0` the files are watched and re-read when they
/// change, so a renewal is picked up without a restart. 0 reads them once.
///
/// Errors here are fatal to the listener that asked for them, deliberately: a
/// server that silently fell back to plaintext on a bad certificate path would
/// be advertising a secure port that is not. That applies only to the *first*
/// load — see `reload_if_changed` for why a later failure must not.
pub fn acceptor_from_files(
    cert_path: &str,
    key_path: &str,
    reload_seconds: u64,
) -> Result<TlsAcceptor> {
    let resolver = Arc::new(ReloadingCert::new(cert_path, key_path)?);

    // `builder_with_provider` rather than `builder()`. The latter reads a
    // process-global default provider that has to be installed exactly once
    // before first use; naming it here means the acceptor cannot depend on
    // whether something else got there first, which in a test binary running
    // many tests in one process is not a hypothetical.
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| OxigeonError::Config(format!("TLS: {e}")))?
    .with_no_client_auth()
    .with_cert_resolver(resolver.clone());

    if reload_seconds > 0 {
        // Polling rather than a filesystem watcher: an interval of minutes is
        // the right granularity for something that changes every few months,
        // two `stat` calls cost nothing, and it needs no platform-specific
        // dependency to behave the same on Windows and Linux.
        let watcher = resolver;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(reload_seconds));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await; // the first tick is immediate; the files were just read
            loop {
                tick.tick().await;
                watcher.reload_if_changed();
            }
        });
    }

    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn load_certs(path: &str) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let file = std::fs::File::open(Path::new(path)).map_err(|e| {
        OxigeonError::Config(format!("TLS: cannot read certificate {path}: {e}"))
    })?;
    let mut reader = std::io::BufReader::new(file);
    let certs: std::result::Result<Vec<_>, _> = rustls_pemfile::certs(&mut reader).collect();
    let certs =
        certs.map_err(|e| OxigeonError::Config(format!("TLS: bad certificate {path}: {e}")))?;

    if certs.is_empty() {
        return Err(OxigeonError::Config(format!(
            "TLS: {path} contains no CERTIFICATE blocks — a PEM chain was expected"
        )));
    }
    Ok(certs)
}

fn load_key(path: &str) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let file = std::fs::File::open(Path::new(path))
        .map_err(|e| OxigeonError::Config(format!("TLS: cannot read key {path}: {e}")))?;
    let mut reader = std::io::BufReader::new(file);
    // Handles PKCS#8, PKCS#1 and SEC1 alike, so an operator does not have to
    // know which `openssl` incantation produced their key.
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| OxigeonError::Config(format!("TLS: bad key {path}: {e}")))?
        .ok_or_else(|| {
            OxigeonError::Config(format!(
                "TLS: {path} contains no private key — expected a PRIVATE KEY, \
                 RSA PRIVATE KEY or EC PRIVATE KEY block"
            ))
        })
}

/// How long a peer gets to finish the TLS handshake.
///
/// A socket opened to a TLS port that then says nothing would otherwise hold a
/// task forever — the same hazard the WebSocket upgrade has, and the same
/// answer.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Complete the TLS handshake, or hand the stream back untouched.
///
/// Bounded, like the WebSocket upgrade: a peer that opens a socket to a TLS
/// port and then says nothing would otherwise hold a task forever.
pub async fn wrap(
    stream: TcpStream,
    acceptor: Option<&TlsAcceptor>,
    timeout: std::time::Duration,
) -> io::Result<MaybeTls> {
    match acceptor {
        None => Ok(MaybeTls::Plain(stream)),
        Some(acceptor) => match tokio::time::timeout(timeout, acceptor.accept(stream)).await {
            Ok(Ok(s)) => Ok(MaybeTls::Tls(Box::new(s))),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TLS handshake timed out",
            )),
        },
    }
}
