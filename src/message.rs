//! Functions and errors for handling messages.

use std::mem;

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

#[cfg(test)]
mod tests {
    use std::mem;

    use bincode::Options;

    use crate::{frame::channel, RequestCode};

    #[tokio::test]
    async fn send_and_recv() {
        const TEST_PORT: u16 = 60191;

        let (mut server_send, _server_recv, _client_send, mut client_recv) =
            channel(TEST_PORT).await;

        let mut buf = Vec::new();
        super::send_request(&mut server_send, &mut buf, RequestCode::ReloadTi, ())
            .await
            .unwrap();
        assert!(buf.is_empty());
        let (code, body) = super::recv_request_raw(&mut client_recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, RequestCode::ReloadTi);
        assert!(body.is_empty());

        buf.clear();
        super::send_forward_request(
            &mut server_send,
            &mut buf,
            "agent@host",
            RequestCode::ReloadTi,
            (),
        )
        .await
        .unwrap();
        let (code, body) = super::recv_request_raw(&mut client_recv, &mut buf)
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
}
