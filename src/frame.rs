//! Functions and errors for handling length-delimited frames.

use std::{mem, num::TryFromIntError};

use bincode::Options;
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The error type for receiving and deserializing a frame.
#[derive(Debug, Error)]
pub enum RecvError {
    #[error("failed deserializing message")]
    DeserializationFailure(#[from] bincode::Error),
    #[error("failed to read from a stream")]
    ReadError(#[from] quinn::ReadExactError),
}

/// Receives and deserializes a message with a big-endian 4-byte length header.
///
/// `buf` will be filled with the message data excluding the 4-byte length
/// header.
///
/// # Errors
///
/// * `RecvError::DeserializationFailure`: if the message could not be
///   deserialized
/// * `RecvError::ReadError`: if the message could not be read
pub async fn recv<'b, T>(recv: &mut RecvStream, buf: &'b mut Vec<u8>) -> Result<T, RecvError>
where
    T: Deserialize<'b>,
{
    recv_raw(recv, buf).await?;
    Ok(bincode::DefaultOptions::new().deserialize(buf)?)
}

/// Receives a sequence of bytes with a big-endian 4-byte length header.
///
/// `buf` will be filled with the message data excluding the 4-byte length
/// header.
///
/// # Errors
///
/// * `quinn::ReadExactError`: if the message could not be read
pub async fn recv_raw<'b>(
    recv: &mut RecvStream,
    buf: &mut Vec<u8>,
) -> Result<(), quinn::ReadExactError> {
    let mut len_buf = [0; mem::size_of::<u32>()];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    buf.resize(len, 0);
    recv.read_exact(buf.as_mut_slice()).await?;
    Ok(())
}

/// The error type for sending a message as a frame.
#[derive(Debug, Error)]
pub enum SendError {
    #[error("failed serializing message")]
    SerializationFailure(#[from] bincode::Error),
    #[error("message is too large")]
    MessageTooLarge(#[from] TryFromIntError),
    #[error("failed to write to a stream")]
    WriteError(#[from] quinn::WriteError),
}

/// Sends a message as a stream of bytes with a big-endian 4-byte length header.
///
/// `buf` will be cleared after the message is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure`: if the message could not be serialized
/// * `SendError::MessageTooLarge`: if the message is too large
/// * `SendError::WriteError`: if the message could not be written
pub async fn send<T>(send: &mut SendStream, buf: &mut Vec<u8>, msg: T) -> Result<(), SendError>
where
    T: Serialize,
{
    buf.resize(mem::size_of::<u32>(), 0);
    bincode::DefaultOptions::new().serialize_into(&mut *buf, &msg)?;
    let len = u32::try_from(buf.len() - 4)?;
    buf[..mem::size_of::<u32>()].clone_from_slice(&len.to_be_bytes());
    send.write_all(buf).await?;
    buf.clear();
    Ok(())
}

/// Sends a sequence of bytes with a big-endian 4-byte length header.
///
/// # Errors
///
/// * `SendError::MessageTooLarge`: if the message is too large
/// * `SendError::WriteError`: if the message could not be written
pub async fn send_raw(send: &mut SendStream, buf: &[u8]) -> Result<(), SendError> {
    let len = u32::try_from(buf.len())?;
    send.write_all(&len.to_be_bytes()).await?;
    send.write_all(buf).await?;
    Ok(())
}

/// Creates a bidirectional channel, returning server's send and receive and
/// client's send and receive streams.
#[cfg(test)]
pub(crate) async fn channel(port: u16) -> (SendStream, RecvStream, SendStream, RecvStream) {
    use std::net::{IpAddr, Ipv6Addr, SocketAddr};

    use futures::StreamExt;
    use quinn::{ClientConfig, Endpoint, ServerConfig};

    const TEST_SERVER_NAME: &str = "test-server";

    let cert =
        rcgen::generate_simple_self_signed([TEST_SERVER_NAME.to_string()]).expect("infallible");
    let cert_chain = vec![rustls::Certificate(
        cert.serialize_der().expect("infallible"),
    )];
    let key_der = rustls::PrivateKey(cert.serialize_private_key_der());
    let server_config = ServerConfig::with_single_cert(cert_chain, key_der).expect("infallible");
    let server_addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port);
    let (_server_endpoint, mut incoming) = Endpoint::server(server_config, server_addr).unwrap();

    let mut root_cert_store = rustls::RootCertStore::empty();
    root_cert_store.add_parsable_certificates(&[cert.serialize_der().expect("infallible")]);
    let client_endpoint =
        Endpoint::client(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)).unwrap();
    let client_config = ClientConfig::with_root_certificates(root_cert_store);
    let client_connecting = client_endpoint
        .connect_with(client_config, server_addr, TEST_SERVER_NAME)
        .unwrap();

    let server_connecting = incoming.next().await.unwrap();

    let client_new_connection = client_connecting.await.unwrap();

    let mut server_new_connection = server_connecting.await.unwrap();

    let (mut client_send, client_recv) = client_new_connection.connection.open_bi().await.unwrap();
    client_send.write_all(b"ready").await.unwrap();

    let (server_send, mut server_recv) = server_new_connection
        .bi_streams
        .next()
        .await
        .unwrap()
        .unwrap();
    let mut server_buf = [0; 5];
    server_recv.read_exact(&mut server_buf).await.unwrap();

    (server_send, server_recv, client_send, client_recv)
}

#[cfg(test)]
mod tests {
    use super::channel;

    #[tokio::test]
    async fn send_and_recv() {
        const TEST_PORT: u16 = 60190;

        let (mut server_send, _server_recv, _client_send, mut client_recv) =
            channel(TEST_PORT).await;

        let mut buf = Vec::new();
        super::send(&mut server_send, &mut buf, "hello")
            .await
            .unwrap();
        assert_eq!(buf.len(), 0);
        super::recv_raw(&mut client_recv, &mut buf).await.unwrap();
        assert_eq!(buf[0] as usize, "hello".len());
        assert_eq!(&buf[1..], b"hello");

        super::send_raw(&mut server_send, b"world").await.unwrap();
        super::recv_raw(&mut client_recv, &mut buf).await.unwrap();
        assert_eq!(buf, b"world");

        super::send(&mut server_send, &mut buf, "hello")
            .await
            .unwrap();
        assert!(buf.is_empty());
        let received = super::recv::<&str>(&mut client_recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(received, "hello");
    }
}
