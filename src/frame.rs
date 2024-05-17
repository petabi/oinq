//! Functions and errors for handling length-delimited frames.

use bincode::Options;
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use std::{io, mem};

/// Receives and deserializes a message with a big-endian 4-byte length header.
///
/// `buf` will be filled with the message data excluding the 4-byte length
/// header.
///
/// # Errors
///
/// Returns an error if the message could not be read or deserialized.
pub async fn recv<'b, T>(recv: &mut RecvStream, buf: &'b mut Vec<u8>) -> io::Result<T>
where
    T: Deserialize<'b>,
{
    recv_raw(recv, buf).await?;
    bincode::DefaultOptions::new()
        .deserialize(buf)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed deserializing message: {e}"),
            )
        })
}

/// Receives a sequence of bytes with a big-endian 4-byte length header.
///
/// `buf` will be filled with the message data excluding the 4-byte length
/// header.
///
/// # Errors
///
/// Returns an error if the message could not be read.
pub async fn recv_raw<'b>(recv: &mut RecvStream, buf: &mut Vec<u8>) -> io::Result<()> {
    let mut len_buf = [0; mem::size_of::<u32>()];
    if let Err(e) = recv.read_exact(&mut len_buf).await {
        return Err(from_read_exact_error_to_io_error(e));
    }
    let len = u32::from_be_bytes(len_buf) as usize;

    buf.resize(len, 0);
    recv.read_exact(buf.as_mut_slice())
        .await
        .map_err(from_read_exact_error_to_io_error)
}

fn from_read_exact_error_to_io_error(e: quinn::ReadExactError) -> io::Error {
    match e {
        quinn::ReadExactError::FinishedEarly(_) => io::Error::from(io::ErrorKind::UnexpectedEof),
        quinn::ReadExactError::ReadError(e) => e.into(),
    }
}

/// Sends a message as a stream of bytes with a big-endian 4-byte length header.
///
/// `buf` will be cleared after the message is sent.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidData`] if the message is too large, or
/// other errors if the message could not be written to the stream.
pub async fn send<T>(send: &mut SendStream, buf: &mut Vec<u8>, msg: T) -> io::Result<()>
where
    T: Serialize,
{
    buf.resize(mem::size_of::<u32>(), 0);
    bincode::DefaultOptions::new()
        .serialize_into(&mut *buf, &msg)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len =
        u32::try_from(buf.len() - 4).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    buf[..mem::size_of::<u32>()].clone_from_slice(&len.to_be_bytes());
    send.write_all(buf).await?;
    buf.clear();
    Ok(())
}

/// Sends a sequence of bytes with a big-endian 4-byte length header.
///
/// # Errors
///
/// Returns an error if the message is too large or could not be written.
pub async fn send_raw(send: &mut SendStream, buf: &[u8]) -> io::Result<()> {
    let len =
        u32::try_from(buf.len()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    send.write_all(&len.to_be_bytes()).await?;
    send.write_all(buf).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn send_and_recv() {
        use crate::test::{channel, TOKEN};

        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let mut buf = Vec::new();
        super::send(&mut channel.server.send, &mut buf, "hello")
            .await
            .unwrap();
        assert_eq!(buf.len(), 0);
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf[0] as usize, "hello".len());
        assert_eq!(&buf[1..], b"hello");

        super::send_raw(&mut channel.server.send, b"world")
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, b"world");

        super::send(&mut channel.server.send, &mut buf, "hello")
            .await
            .unwrap();
        assert!(buf.is_empty());
        let received = super::recv::<&str>(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(received, "hello");
    }
}
