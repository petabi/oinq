//! Functions and errors for handling messages.

use crate::frame;
use bincode::Options;
use quinn::{RecvStream, SendStream};
use serde::Serialize;
use std::{fmt, io, mem};

/// Receives a message as a stream of bytes with a big-endian 4-byte length
/// header.
///
/// `buf` will be filled with the message.
///
/// # Errors
///
/// Returns an error if the message could not be read or deserialized.
///
/// # Panics
///
/// * panic if it failed to convert 4 byte data to u32 value
pub async fn recv_request_raw<'b>(
    recv: &mut RecvStream,
    buf: &'b mut Vec<u8>,
) -> io::Result<(u32, &'b [u8])> {
    frame::recv_raw(recv, buf).await?;
    if buf.len() < mem::size_of::<u32>() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "message too short to contain a code",
        ));
    }
    let code = u32::from_le_bytes(buf[..mem::size_of::<u32>()].try_into().expect("4 bytes"));
    Ok((code, buf[mem::size_of::<u32>()..].as_ref()))
}

/// Sends a request with a big-endian 4-byte length header.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidData`] if the message is too large, or
/// other errors if the message could not be written to the stream.
pub async fn send_request<C, B>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    code: C,
    body: B,
) -> io::Result<()>
where
    C: Into<u32>,
    B: Serialize,
{
    use std::io::Write;

    buf.clear();
    buf.extend_from_slice(&code.into().to_le_bytes());
    bincode::DefaultOptions::new()
        .serialize_into(buf as &mut dyn Write, &body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    frame::send_raw(send, buf).await?;
    buf.clear();
    Ok(())
}

/// Sends an `Ok` response.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidData`] if the message is too large, or
/// other errors if the message could not be written to the stream.
pub async fn send_ok<T: Serialize>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    body: T,
) -> io::Result<()> {
    frame::send(send, buf, Ok(body) as Result<T, &str>).await
}

/// Sends an `Err` response.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidData`] if the message is too large, or
/// other errors if the message could not be written to the stream.
pub async fn send_err<E: fmt::Display>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    e: E,
) -> io::Result<()> {
    frame::send(send, buf, Err(format!("{e:#}")) as Result<(), String>).await
}

#[cfg(test)]
mod tests {
    use crate::frame;
    use crate::test::{channel, TOKEN};

    #[tokio::test]
    async fn send_and_recv() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        const CODE: u32 = 0x1234;
        let mut buf = Vec::new();
        super::send_request(&mut channel.server.send, &mut buf, CODE, ())
            .await
            .unwrap();
        assert!(buf.is_empty());
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, CODE);
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
