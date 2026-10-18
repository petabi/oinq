//! This consumer intentionally has no direct Quinn dependency.
use oinq::frame::{FrameLimit, FramedReader, FramedWriter};
use oinq::{
    ApplicationCode, CloseInfo, Connection, Endpoint, Incoming, TransportError, TransportOptions,
};
use rustls::pki_types::CertificateDer;
use std::{
    net::{SocketAddr, UdpSocket},
    sync::Arc,
};
use tokio::io::{AsyncRead, AsyncWrite};

pub fn endpoints(
    server: Arc<rustls::ServerConfig>,
    client: Arc<rustls::ClientConfig>,
    bind: SocketAddr,
) -> Result<(Endpoint, Endpoint), TransportError> {
    Ok((
        Endpoint::server(server, bind, TransportOptions::default())?,
        Endpoint::client(client, bind, TransportOptions::default())?,
    ))
}

pub fn prebound(
    server: Arc<rustls::ServerConfig>,
    client: Arc<rustls::ClientConfig>,
    a: UdpSocket,
    b: UdpSocket,
) -> Result<(Endpoint, Endpoint), TransportError> {
    Ok((
        Endpoint::server_with_socket(server, a, TransportOptions::default())?,
        Endpoint::client_with_socket(client, b, TransportOptions::default())?,
    ))
}

pub async fn establish(incoming: Incoming) -> Result<Connection, TransportError> {
    incoming.await
}
pub fn certificates(connection: &Connection) -> Option<&[CertificateDer<'static>]> {
    connection.peer_certificates()
}
pub async fn closed(connection: &Connection) -> CloseInfo {
    connection.closed().await
}

pub async fn exchange(connection: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    fn writer<T: AsyncWrite + Unpin>(_: &T) {}
    fn reader<T: AsyncRead + Unpin>(_: &T) {}
    let (send, recv) = connection.open_bi().await?;
    writer(&send);
    reader(&recv);
    let limit = FrameLimit::new(4096)?;
    let mut framed = FramedWriter::new(send, limit);
    framed.start_raw(b"hello")?;
    framed.flush().await?;
    framed.into_inner().finish()?;
    let mut framed = FramedReader::new(recv, limit);
    let _ = framed.read().await?;
    framed.into_inner().stop(ApplicationCode::new(1)?)?;
    Ok(())
}
