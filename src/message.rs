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

/// Sends a message with a big-endian 4-byte length header.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_request<T: Serialize>(
    send: &mut SendStream,
    mut buf: &mut Vec<u8>,
    code: RequestCode,
    body: T,
) -> Result<(), SendError> {
    buf.clear();
    bincode::serialize_into(&mut buf, &code)?;
    bincode::DefaultOptions::new().serialize_into(&mut buf, &body)?;
    frame::send_raw(send, buf).await?;
    buf.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use bincode::Options;

    use crate::{frame::channel, RequestCode};

    #[tokio::test]
    async fn send_and_recv() {
        const TEST_PORT: u16 = 60191;

        let (mut server_send, _server_recv, _client_send, mut client_recv) =
            channel(TEST_PORT).await;

        let mut buf = Vec::new();
        super::send_request(&mut server_send, &mut buf, RequestCode::ReloadTi, "hello")
            .await
            .unwrap();
        assert!(buf.is_empty());
        let (code, body) = super::recv_request_raw(&mut client_recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, RequestCode::ReloadTi);
        let msg = bincode::DefaultOptions::new()
            .deserialize::<&str>(body)
            .unwrap();
        assert_eq!(msg, "hello");
    }
}
