//! Functions and errors for handling messages.

use crate::frame::{self, RecvError, SendError};
use bincode::Options;
use quinn::{ConnectionError, RecvStream, SendStream};
use serde::Serialize;
use std::{fmt, mem};
use thiserror::Error;

/// Receives a message as a stream of bytes with a big-endian 4-byte length
/// header.
///
/// `buf` will be filled with the message.
///
/// # Errors
///
/// * `RecvError::DeserializationFailure` if the message could not be
///   deserialized
/// * `RecvError::ReadError` if the message could not be read
///
/// # Panics
///
/// * panic if it failed to convert 4 byte data to u32 value
pub async fn recv_request_raw<'b>(
    recv: &mut RecvStream,
    buf: &'b mut Vec<u8>,
) -> Result<(u32, &'b [u8]), RecvError> {
    frame::recv_raw(recv, buf).await?;
    if buf.len() < mem::size_of::<u32>() {
        return Err(RecvError::DeserializationFailure(Box::new(
            bincode::ErrorKind::SizeLimit,
        )));
    }
    let code = u32::from_le_bytes(buf[..mem::size_of::<u32>()].try_into().expect("4 bytes"));
    Ok((code, buf[mem::size_of::<u32>()..].as_ref()))
}

/// The error type for a handshake failure.
#[derive(Debug, Error)]
pub enum HandshakeError {
    #[error("connection closed by peer")]
    ConnectionClosed,
    #[error("connection lost")]
    ConnectionLost(#[from] ConnectionError),
    #[error("cannot receive a message")]
    ReadError(#[from] quinn::ReadError),
    #[error("cannot send a message")]
    WriteError(#[from] quinn::WriteError),
    #[error("cannot serialize a message")]
    SerializationFailure(#[from] bincode::Error),
    #[error("arguments are too long")]
    MessageTooLarge,
    #[error("invalid message")]
    InvalidMessage,
    #[error("protocol version {0} is not supported; version {1} is required")]
    IncompatibleProtocol(String, String),
}

impl From<SendError> for HandshakeError {
    fn from(e: SendError) -> Self {
        match e {
            SendError::SerializationFailure(e) => HandshakeError::SerializationFailure(e),
            SendError::MessageTooLarge(_) => HandshakeError::MessageTooLarge,
            SendError::WriteError(e) => HandshakeError::WriteError(e),
        }
    }
}

/// Sends a request with a big-endian 4-byte length header.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_request<C, B>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    code: C,
    body: B,
) -> Result<(), SendError>
where
    C: Into<u32>,
    B: Serialize,
{
    buf.clear();
    serialize_request(buf, code, body)?;
    frame::send_raw(send, buf).await?;
    buf.clear();
    Ok(())
}

/// Serializes the given request and appends it to `buf`.
///
/// # Errors
///
/// * `bincode::Error` if the message could not be serialized
fn serialize_request<C, B>(buf: &mut Vec<u8>, code: C, body: B) -> Result<(), bincode::Error>
where
    C: Into<u32>,
    B: Serialize,
{
    buf.extend_from_slice(&code.into().to_le_bytes());
    bincode::DefaultOptions::new().serialize_into(buf, &body)?;
    Ok(())
}

/// Sends an `Ok` response.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_ok<T: Serialize>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    body: T,
) -> Result<(), SendError> {
    frame::send(send, buf, Ok(body) as Result<T, &str>).await?;
    Ok(())
}

/// Sends an `Err` response.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_err<E: fmt::Display>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    e: E,
) -> Result<(), SendError> {
    frame::send(send, buf, Err(format!("{e:#}")) as Result<(), String>).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test::{channel, TOKEN};
    use crate::{frame, RequestCode};

    #[tokio::test]
    async fn send_and_recv() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let mut buf = Vec::new();
        super::send_request(
            &mut channel.server.send,
            &mut buf,
            RequestCode::ReloadTi,
            (),
        )
        .await
        .unwrap();
        assert!(buf.is_empty());
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, u32::from(RequestCode::ReloadTi));
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn send_ok() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let mut buf = Vec::new();
        super::send_ok(&mut channel.server.send, &mut buf, "hello")
            .await
            .unwrap();
        assert!(buf.is_empty());
        let body: Result<&str, &str> = frame::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(body, Ok("hello"));
    }

    #[tokio::test]
    async fn send_err() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let mut buf = Vec::new();
        super::send_err(&mut channel.server.send, &mut buf, "hello")
            .await
            .unwrap();
        assert!(buf.is_empty());
        let body: Result<(), &str> = frame::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(body, Err("hello"));
    }
}
