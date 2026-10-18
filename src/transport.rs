//! Owned QUIC transport over Tokio. TLS policy belongs to the supplied rustls config.
//!
//! Constructors require an entered Tokio runtime with I/O and time enabled. No application
//! tasks are spawned. Connections retain their endpoint; streams retain their connection.
//! Dropping one clone does not close other handles. Explicit endpoint close immediately
//! closes all sessions; it is not an application drain. Final stream drop follows Quinn's
//! FIN/stop behavior, not a promise of reset. Use explicit reset/stop to abort exchanges.
//!
//! Accept and stream-opening operations are cancellation safe. Dropping an `Incoming`
//! abandons that establishment; cancelling `connect` abandons its attempt. Read/write
//! polls can make partial progress; generic framing retains that progress between calls.
//! No application identity, SAN interpretation, service codes, admission, or authorization
//! is provided; these belong to the caller.
//!
//! # TLS configuration
//!
//! Create servers with [`Endpoint::server`] and clients with [`Endpoint::client`],
//! or adopt prebound standard UDP sockets with [`Endpoint::server_with_socket`]
//! and [`Endpoint::client_with_socket`]. Constructors validate [`TransportOptions`].
//! The supplied rustls configuration controls certificate verification,
//! client-certificate enforcement, SNI/server-name checks, and crypto provider
//! selection. A provider without QUIC's required initial cipher suite produces
//! a [`TransportErrorKind::Tls`] error; the provider is not replaced.
//!
//! # Framed streams
//!
//! [`SendStream`] and [`RecvStream`] implement Tokio I/O directly, without an
//! extra payload-copying adapter, and compose with [`crate::frame::FramedWriter`]
//! and [`crate::frame::FramedReader`]. Cancelled framed reads and flushes retain
//! partial progress; resume on the same framed object.
//!
//! ```no_run
//! # async fn exchange(
//! #     connection: &oinq::Connection,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! use oinq::frame::{FrameLimit, FramedReader, FramedWriter};
//!
//! let (send, recv) = connection.open_bi().await?;
//! let limit = FrameLimit::new(1024 * 1024)?;
//! let mut writer = FramedWriter::new(send, limit);
//! writer.start_raw(b"request")?;
//! writer.flush().await?;
//! let mut send = writer.into_inner();
//! send.finish()?;
//! let mut reader = FramedReader::new(recv, limit);
//! let response = reader.read().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Quinn interoperability
//!
//! These wrappers do not expose raw Quinn handles. The legacy helpers in
//! [`crate::frame`], [`crate::message`], and [`crate::request`] that accept raw
//! Quinn streams remain available; using them requires a direct Quinn dependency.

use std::{
    error::Error,
    fmt,
    future::Future,
    io,
    net::{SocketAddr, UdpSocket},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use rustls::pki_types::CertificateDer;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A checked QUIC application error code (0 through 2^62 - 1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApplicationCode(u64);

impl ApplicationCode {
    /// Largest application error code representable by QUIC.
    pub const MAX: u64 = (1 << 62) - 1;

    /// Checks the QUIC application-code range.
    ///
    /// # Errors
    /// Returns an invalid-option error for values above [`Self::MAX`].
    pub fn new(value: u64) -> Result<Self, TransportError> {
        varint(value)?;
        Ok(Self(value))
    }

    /// Returns the checked application error code.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    fn quinn(self) -> quinn::VarInt {
        // The private field is checked at construction.
        quinn::VarInt::from_u64(self.0).expect("checked application code")
    }
}

/// Broad categories that do not depend on the transport implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransportErrorKind {
    /// A transport option or application code is outside its allowed range.
    InvalidOptions,
    /// The supplied TLS configuration cannot be used for QUIC.
    Tls,
    /// A socket operation failed.
    Io,
    /// A connection attempt could not be started.
    Connect,
    /// Connection establishment or an established connection failed.
    Connection,
    /// A stream operation failed.
    Stream,
}

/// Structured operation failure, retaining the original error as its source.
#[derive(Debug)]
pub struct TransportError {
    kind: TransportErrorKind,
    source: Box<dyn Error + Send + Sync>,
}

impl TransportError {
    /// Returns the implementation-independent error category.
    #[must_use]
    pub fn kind(&self) -> TransportErrorKind {
        self.kind
    }

    fn new(kind: TransportErrorKind, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind,
            source: Box::new(source),
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.source)
    }
}

impl Error for TransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

fn varint(value: u64) -> Result<quinn::VarInt, TransportError> {
    quinn::VarInt::from_u64(value)
        .map_err(|e| TransportError::new(TransportErrorKind::InvalidOptions, e))
}

/// Transport limits. Defaults: no keepalive, 30s idle timeout, 100 incoming
/// streams per direction, a 1,250,000-byte per-stream receive window, and
/// 10,000,000-byte connection receive and send windows. Stream counts are limited
/// to 2^60; receive windows and timeout milliseconds must fit 62 bits.
/// `None` or a timeout below one millisecond disables the local idle timeout;
/// keepalive must be positive and shorter than an enabled idle timeout.
/// Peers negotiate the effective idle timeout.
#[derive(Clone, Debug)]
pub struct TransportOptions {
    /// Periodic traffic to prevent idle expiry; disabled by default.
    pub keep_alive_interval: Option<Duration>,
    /// Maximum inactivity, truncated to milliseconds.
    /// `None` or a value below one millisecond disables the local limit.
    pub idle_timeout: Option<Duration>,
    /// Incoming bidirectional stream credit. Zero prevents opening streams.
    pub max_concurrent_bidi_streams: u64,
    /// Incoming unidirectional stream credit. Zero prevents opening streams.
    pub max_concurrent_uni_streams: u64,
    /// Per-stream receive credit in bytes.
    pub stream_receive_window: u64,
    /// Shared connection receive credit in bytes.
    pub receive_window: u64,
    /// Maximum buffered unacknowledged outgoing bytes; full `u64` range.
    pub send_window: u64,
}

impl Default for TransportOptions {
    fn default() -> Self {
        Self {
            keep_alive_interval: None,
            idle_timeout: Some(Duration::from_secs(30)),
            max_concurrent_bidi_streams: 100,
            max_concurrent_uni_streams: 100,
            stream_receive_window: 1_250_000,
            receive_window: 10_000_000,
            send_window: 10_000_000,
        }
    }
}

impl TransportOptions {
    /// Validates all options without creating an endpoint.
    ///
    /// # Errors
    /// Returns an invalid-option error for an unrepresentable limit or invalid keepalive.
    pub fn validate(&self) -> Result<(), TransportError> {
        self.build().map(|_| ())
    }

    fn build(&self) -> Result<quinn::TransportConfig, TransportError> {
        if self.max_concurrent_bidi_streams > (1 << 60)
            || self.max_concurrent_uni_streams > (1 << 60)
        {
            return Err(TransportError::new(
                TransportErrorKind::InvalidOptions,
                io::Error::new(io::ErrorKind::InvalidInput, "stream count exceeds 2^60"),
            ));
        }
        // QUIC encodes idle timeouts in whole milliseconds; zero disables the limit.
        let idle_timeout = self
            .idle_timeout
            .map(|idle| Duration::new(idle.as_secs(), idle.subsec_millis() * 1_000_000))
            .filter(|idle| !idle.is_zero());
        if self
            .keep_alive_interval
            .is_some_and(|v| v.is_zero() || idle_timeout.is_some_and(|idle| v >= idle))
        {
            return Err(TransportError::new(
                TransportErrorKind::InvalidOptions,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "keepalive must be positive and shorter than idle timeout",
                ),
            ));
        }
        if let Some(interval) = self.keep_alive_interval {
            quinn::IdleTimeout::try_from(interval)
                .map_err(|e| TransportError::new(TransportErrorKind::InvalidOptions, e))?;
        }
        let idle = idle_timeout
            .map(TryInto::try_into)
            .transpose()
            .map_err(|e| TransportError::new(TransportErrorKind::InvalidOptions, e))?;
        let mut config = quinn::TransportConfig::default();
        config
            .keep_alive_interval(self.keep_alive_interval)
            .max_idle_timeout(idle)
            .max_concurrent_bidi_streams(varint(self.max_concurrent_bidi_streams)?)
            .max_concurrent_uni_streams(varint(self.max_concurrent_uni_streams)?)
            .stream_receive_window(varint(self.stream_receive_window)?)
            .receive_window(varint(self.receive_window)?)
            .send_window(self.send_window);
        Ok(config)
    }
}

/// Cloneable endpoint handle. Explicit close affects all its connections.
#[derive(Clone, Debug)]
pub struct Endpoint {
    inner: quinn::Endpoint,
    options: TransportOptions,
}

fn server_config(
    tls: Arc<rustls::ServerConfig>,
    options: &TransportOptions,
) -> Result<quinn::ServerConfig, TransportError> {
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)
        .map_err(|e| TransportError::new(TransportErrorKind::Tls, e))?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(Arc::new(options.build()?));
    Ok(config)
}

impl Endpoint {
    /// Binds a server using the supplied TLS policy.
    ///
    /// # Errors
    /// Returns TLS, option, or socket errors.
    pub fn server(
        tls: Arc<rustls::ServerConfig>,
        bind: SocketAddr,
        options: TransportOptions,
    ) -> Result<Self, TransportError> {
        Self::server_with_socket(tls, UdpSocket::bind(bind).map_err(io_error)?, options)
    }

    /// Adopts a prebound server socket. Requires an entered Tokio runtime.
    ///
    /// # Errors
    /// Returns TLS, option, or socket errors.
    pub fn server_with_socket(
        tls: Arc<rustls::ServerConfig>,
        socket: UdpSocket,
        options: TransportOptions,
    ) -> Result<Self, TransportError> {
        let config = server_config(tls, &options)?;
        let inner = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(config),
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(io_error)?;
        Ok(Self { inner, options })
    }

    /// Binds a client using the supplied verification and crypto policy.
    ///
    /// # Errors
    /// Returns TLS, option, or socket errors.
    pub fn client(
        tls: Arc<rustls::ClientConfig>,
        bind: SocketAddr,
        options: TransportOptions,
    ) -> Result<Self, TransportError> {
        Self::client_with_socket(tls, UdpSocket::bind(bind).map_err(io_error)?, options)
    }

    /// Adopts a prebound client socket. Requires an entered Tokio runtime.
    ///
    /// # Errors
    /// Returns TLS, option, or socket errors.
    pub fn client_with_socket(
        tls: Arc<rustls::ClientConfig>,
        socket: UdpSocket,
        options: TransportOptions,
    ) -> Result<Self, TransportError> {
        let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .map_err(|e| TransportError::new(TransportErrorKind::Tls, e))?;
        let mut config = quinn::ClientConfig::new(Arc::new(crypto));
        config.transport_config(Arc::new(options.build()?));
        let mut inner = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            None,
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(io_error)?;
        inner.set_default_client_config(config);
        Ok(Self { inner, options })
    }

    /// Waits for an independently owned establishment attempt. Individual handshake
    /// failures are returned by [`Incoming`], never as listener termination.
    /// Await each attempt independently and continue accepting after a failed handshake.
    /// Returns `None` when the endpoint is closed. Cancelling this wait does not
    /// consume an establishment attempt.
    pub async fn accept(&self) -> Option<Incoming> {
        let incoming = self.inner.accept().await?;
        let endpoint = self.clone();
        Some(Incoming {
            future: Box::pin(async move {
                let inner = incoming.await.map_err(connection_error)?;
                Ok(Connection::new(inner, endpoint))
            }),
        })
    }

    /// Establishes a connection, preserving SNI and server-name verification
    /// according to the supplied TLS policy. Cancelling abandons this attempt.
    ///
    /// # Errors
    /// Returns address, name, or handshake errors.
    pub async fn connect(
        &self,
        remote_addr: SocketAddr,
        server_name: &str,
    ) -> Result<Connection, TransportError> {
        let inner = self
            .inner
            .connect(remote_addr, server_name)
            .map_err(|e| TransportError::new(TransportErrorKind::Connect, e))?
            .await
            .map_err(connection_error)?;
        Ok(Connection::new(inner, self.clone()))
    }

    /// Replaces TLS policy for future connections only. Established sessions retain
    /// their negotiated policy and peer certificates.
    ///
    /// # Errors
    /// Returns TLS or option errors, leaving the old configuration installed.
    pub fn set_server_config(&self, tls: Arc<rustls::ServerConfig>) -> Result<(), TransportError> {
        self.inner
            .set_server_config(Some(server_config(tls, &self.options)?));
        Ok(())
    }

    /// Returns the bound socket address.
    ///
    /// # Errors
    /// Returns socket inspection errors.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        self.inner.local_addr().map_err(io_error)
    }

    /// Closes immediately, including all existing connections. No application drain.
    pub fn close(&self, code: ApplicationCode, reason: &[u8]) {
        self.inner.close(code.quinn(), reason);
    }

    /// Waits until all endpoint connections have completed transport shutdown.
    pub async fn wait_idle(&self) {
        self.inner.wait_idle().await;
    }
}

fn io_error(e: io::Error) -> TransportError {
    TransportError::new(TransportErrorKind::Io, e)
}
fn connection_error(e: quinn::ConnectionError) -> TransportError {
    TransportError::new(TransportErrorKind::Connection, e)
}

/// Owned establishment future. It can outlive the endpoint handle that accepted it.
/// Dropping it abandons the attempt. A handshake failure affects only this attempt;
/// the endpoint can continue accepting connections.
#[must_use = "incoming connections must be awaited to establish them"]
pub struct Incoming {
    future: Pin<Box<dyn Future<Output = Result<Connection, TransportError>> + Send>>,
}

impl Future for Incoming {
    type Output = Result<Connection, TransportError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.future.as_mut().poll(cx)
    }
}

/// The reason a connection closed.
#[derive(Debug)]
#[non_exhaustive]
pub enum CloseInfo {
    /// The peer closed the connection with an application error.
    Application {
        /// Application error code supplied by the peer.
        code: ApplicationCode,
        /// Uninterpreted reason bytes supplied by the peer.
        reason: Vec<u8>,
    },
    /// The local application closed the connection or endpoint.
    Local,
    /// Closure due to a transport failure, including timeout or peer transport errors.
    Transport(TransportError),
}

/// Clones refer to the same connection. Concurrent acceptors compete for streams;
/// they do not receive copies. Every handle retains the endpoint resources.
#[derive(Clone, Debug)]
pub struct Connection {
    state: Arc<ConnectionInner>,
}

#[derive(Debug)]
struct ConnectionInner {
    connection: quinn::Connection,
    endpoint: Endpoint,
    certificates: Option<Arc<[CertificateDer<'static>]>>,
}

impl Connection {
    fn new(inner: quinn::Connection, endpoint: Endpoint) -> Self {
        let certificates = inner
            .peer_identity()
            .and_then(|v| v.downcast::<Vec<CertificateDer<'static>>>().ok())
            .map(|v| Arc::from(*v));
        Self {
            state: Arc::new(ConnectionInner {
                connection: inner,
                endpoint,
                certificates,
            }),
        }
    }

    /// Returns the peer's chain in TLS order, without interpreting identity.
    #[must_use]
    pub fn peer_certificates(&self) -> Option<&[CertificateDer<'static>]> {
        self.state.certificates.as_deref()
    }

    /// Returns the local address, using the connection's local IP when available.
    ///
    /// # Errors
    /// Returns socket inspection errors.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        let mut addr = self.state.endpoint.local_addr()?;
        if let Some(ip) = self.state.connection.local_ip() {
            addr.set_ip(ip);
        }
        Ok(addr)
    }

    /// Returns the peer's current socket address.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.state.connection.remote_address()
    }

    /// Opens a bidirectional stream, cancellation safely.
    /// The peer observes it after data or FIN is sent.
    ///
    /// # Errors
    /// Returns a connection error if the connection closes.
    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream), TransportError> {
        let (send, recv) = self
            .state
            .connection
            .open_bi()
            .await
            .map_err(connection_error)?;
        Ok((self.send(send), self.recv(recv)))
    }

    /// Accepts the next bidirectional stream, cancellation safely.
    ///
    /// # Errors
    /// Returns a connection error if the connection closes.
    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream), TransportError> {
        let (send, recv) = self
            .state
            .connection
            .accept_bi()
            .await
            .map_err(connection_error)?;
        Ok((self.send(send), self.recv(recv)))
    }

    /// Opens a unidirectional stream, cancellation safely.
    /// The peer observes it after data or FIN is sent.
    ///
    /// # Errors
    /// Returns a connection error if the connection closes.
    pub async fn open_uni(&self) -> Result<SendStream, TransportError> {
        self.state
            .connection
            .open_uni()
            .await
            .map(|v| self.send(v))
            .map_err(connection_error)
    }

    /// Accepts a unidirectional stream, cancellation safely.
    ///
    /// # Errors
    /// Returns a connection error if the connection closes.
    pub async fn accept_uni(&self) -> Result<RecvStream, TransportError> {
        self.state
            .connection
            .accept_uni()
            .await
            .map(|v| self.recv(v))
            .map_err(connection_error)
    }

    fn send(&self, inner: quinn::SendStream) -> SendStream {
        SendStream {
            inner,
            reset: false,
            _connection: self.clone(),
        }
    }
    fn recv(&self, inner: quinn::RecvStream) -> RecvStream {
        RecvStream {
            inner,
            _connection: self.clone(),
        }
    }

    /// Immediately closes the shared connection with an application code and byte reason.
    pub fn close(&self, code: ApplicationCode, reason: &[u8]) {
        self.state.connection.close(code.quinn(), reason);
    }

    /// Waits for closure; cancellation does not close the connection.
    pub async fn closed(&self) -> CloseInfo {
        match self.state.connection.closed().await {
            quinn::ConnectionError::ApplicationClosed(v) => CloseInfo::Application {
                code: ApplicationCode(v.error_code.into_inner()),
                reason: v.reason.to_vec(),
            },
            quinn::ConnectionError::LocallyClosed => CloseInfo::Local,
            e => CloseInfo::Transport(connection_error(e)),
        }
    }
}

/// Tokio writer directly wrapping a QUIC stream, without a payload adapter.
#[derive(Debug)]
pub struct SendStream {
    inner: quinn::SendStream,
    reset: bool,
    _connection: Connection,
}

impl SendStream {
    /// Synchronously queues FIN after buffered data; distinct from reset.
    ///
    /// # Errors
    /// Returns a stream error if already finished or reset.
    pub fn finish(&mut self) -> Result<(), TransportError> {
        self.inner
            .finish()
            .map_err(|e| TransportError::new(TransportErrorKind::Stream, e))
    }

    /// Abandons outstanding data with an application code.
    ///
    /// # Errors
    /// Returns a stream error if already closed.
    pub fn reset(&mut self, code: ApplicationCode) -> Result<(), TransportError> {
        self.inner
            .reset(code.quinn())
            .map_err(|e| TransportError::new(TransportErrorKind::Stream, e))?;
        self.reset = true;
        Ok(())
    }

    /// Waits for peer stop (`Some(code)`) or acknowledgment of all data and FIN
    /// (`None`). Neither proves application processing or delivery. Use an application
    /// response when confirmation is required. Cancellation safe.
    ///
    /// # Errors
    /// Returns transport failure; reset is not successful FIN completion.
    pub async fn stopped(&self) -> Result<Option<ApplicationCode>, TransportError> {
        if self.reset {
            return Err(TransportError::new(
                TransportErrorKind::Stream,
                io::Error::new(io::ErrorKind::ConnectionAborted, "stream was locally reset"),
            ));
        }
        self.inner
            .stopped()
            .await
            .map(|v| v.map(|v| ApplicationCode(v.into_inner())))
            .map_err(|e| TransportError::new(TransportErrorKind::Stream, e))
    }
}

impl AsyncWrite for SendStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.inner), cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.inner), cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.inner), cx)
    }
}

/// Tokio reader retaining its connection and endpoint resources.
#[derive(Debug)]
pub struct RecvStream {
    inner: quinn::RecvStream,
    _connection: Connection,
}

impl RecvStream {
    /// Stops receiving and asks the peer to stop sending.
    ///
    /// # Errors
    /// Returns a stream error if already closed.
    pub fn stop(&mut self, code: ApplicationCode) -> Result<(), TransportError> {
        self.inner
            .stop(code.quinn())
            .map_err(|e| TransportError::new(TransportErrorKind::Stream, e))
    }
}

impl AsyncRead for RecvStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.inner), cx, buf)
    }
}
