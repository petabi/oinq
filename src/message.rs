//! Functions and errors for handling messages.

use std::{fmt, mem};

use bincode::Options;
use quinn::{RecvStream, SendStream};
use serde::Serialize;

use crate::{
    frame::{self, RecvError, SendError},
    RequestCode,
};

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
pub async fn recv_request_raw<'b>(
    recv: &mut RecvStream,
    buf: &'b mut Vec<u8>,
) -> Result<(RequestCode, &'b [u8]), RecvError> {
    frame::recv_raw(recv, buf).await?;
    if buf.len() < mem::size_of::<RequestCode>() {
        return Err(RecvError::DeserializationFailure(Box::new(
            bincode::ErrorKind::SizeLimit,
        )));
    }
    let code = bincode::deserialize(&buf[..mem::size_of::<RequestCode>()])?;
    Ok((code, buf[mem::size_of::<RequestCode>()..].as_ref()))
}

/// Sends a request with a big-endian 4-byte length header.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_request<T: Serialize>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    code: RequestCode,
    body: T,
) -> Result<(), SendError> {
    buf.clear();
    serialize_request(buf, code, body)?;
    frame::send_raw(send, buf).await?;
    buf.clear();
    Ok(())
}

/// Sends a request encapsulated in a `RequestCode::Forward` request, with a
/// big-endian 4-byte length header.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_forward_request<T: Serialize>(
    send: &mut SendStream,
    mut buf: &mut Vec<u8>,
    dst: &str,
    code: RequestCode,
    body: T,
) -> Result<(), SendError> {
    buf.clear();
    bincode::serialize_into(&mut buf, &RequestCode::Forward)?;
    let codec = bincode::DefaultOptions::new();
    codec.serialize_into(&mut buf, dst)?;
    let len = codec.serialized_size(&body)? + mem::size_of::<RequestCode>() as u64;
    codec.serialize_into(&mut buf, &len)?;
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
fn serialize_request<T: Serialize>(
    mut buf: &mut Vec<u8>,
    code: RequestCode,
    body: T,
) -> Result<(), bincode::Error> {
    bincode::serialize_into(&mut buf, &code)?;
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
    frame::send(send, buf, Err(format!("{:#}", e)) as Result<(), String>).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::mem;

    use bincode::Options;

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
        assert_eq!(code, RequestCode::ReloadTi);
        assert!(body.is_empty());

        buf.clear();
        super::send_forward_request(
            &mut channel.server.send,
            &mut buf,
            "agent@host",
            RequestCode::ReloadTi,
            (),
        )
        .await
        .unwrap();
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, RequestCode::Forward);
        let (dst, msg) = bincode::DefaultOptions::new()
            .deserialize::<(String, &[u8])>(body)
            .unwrap();
        assert_eq!(dst, "agent@host");
        assert_eq!(msg.len(), mem::size_of::<RequestCode>());
        let code =
            bincode::deserialize::<RequestCode>(&msg[..mem::size_of::<RequestCode>()]).unwrap();
        assert_eq!(code, RequestCode::ReloadTi);
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
