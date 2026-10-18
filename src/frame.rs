//! Length-delimited framing with a four-byte big-endian payload length.
//!
//! [`FramedReader`] and [`FramedWriter`] own generic Tokio I/O and require a
//! caller-selected [`FrameLimit`]. Quinn streams implement the required Tokio
//! traits directly. The original Quinn free functions remain available with
//! their existing signatures and wire behavior; bounded framing is opt-in.
//!
//! # Limits and decoding
//!
//! Protocol implementations choose a required payload limit, excluding the
//! four-byte header. Zero-length payloads are valid; limits above `u32::MAX`
//! are rejected. Readers check incoming declarations before growing the payload
//! buffer. Writers check raw payload sizes and cap serialization before sending
//! any header bytes.
//!
//! Serialized bodies use bincode 2's standard configuration with serde.
//! [`Frame::decode`] permits trailing bytes for prefix decoding;
//! [`Frame::decode_strict`] requires the entire payload to be consumed.
//! Both apply the configured limit to collection allocation claims and
//! string/byte storage, including nested collections. This conservative budget
//! can reject values whose in-memory representation exceeds their encoded size;
//! choose a limit that accommodates both. Custom `Deserialize` implementations
//! remain responsible for their own allocations. Decode errors affect only the
//! owned frame; the reader remains aligned for the next frame.
//!
//! # Ownership and buffer reuse
//!
//! Each read returns an owned [`Frame`]. Decoded values may borrow from it, and
//! subsequent reads do not invalidate its bytes. Return its allocation with
//! [`Frame::into_bytes`] and [`FramedReader::recycle`] between frames to reuse
//! it; without recycling, the next frame gets a new buffer. Writers retain and
//! reuse their serialization buffer.
//!
//! # EOF, errors, and cancellation
//!
//! [`FramedReader::read`] returns `None` for EOF before a new header, and
//! [`FrameError::Truncated`] for EOF within a header or payload. Transport
//! errors preserve their original [`std::io::Error`] and source, including Quinn
//! reset details. Read errors, including an excessive declaration, poison the
//! reader. Write and flush I/O errors poison the writer. Later operations return
//! [`FrameError::Poisoned`] to prevent reuse after framing alignment is lost.
//! Serialization and outgoing size errors occur before I/O and leave the writer
//! usable.
//!
//! Cancelling a read or flush future retains partial progress in the object.
//! Resume with [`FramedReader::read`] or [`FramedWriter::flush`]. A pending write
//! cannot be replaced, even with identical bytes. Cancellation does not abort an
//! application exchange. Dropping a framed object drops its I/O object; generic
//! Tokio I/O alone cannot guarantee a QUIC reset. Extracting the I/O with
//! `into_inner` discards framing state, so callers must account for partial frames.
//!
//! # Example
//!
//! ```
//! # async fn example() -> Result<(), oinq::frame::FrameError> {
//! use oinq::frame::{FrameLimit, FramedReader, FramedWriter};
//!
//! let limit = FrameLimit::new(1024)?; // Caller-selected example policy.
//! let mut writer = FramedWriter::new(Vec::new(), limit);
//! writer.start("hello")?;
//! writer.flush().await?; // Resume this method if the future is cancelled.
//! let bytes = writer.into_inner();
//! let mut reader = FramedReader::new(bytes.as_slice(), limit);
//! if let Some(frame) = reader.read().await? {
//!     assert_eq!(frame.decode_strict::<&str>()?, "hello");
//!     reader.recycle(frame.into_bytes())?;
//! }
//! assert!(reader.read().await?.is_none());
//! # Ok(())
//! # }
//! ```

mod bounded;
mod decode;

use std::{io, mem};

pub use bounded::{Frame, FrameError, FrameLimit, FramePart, FramedReader, FramedWriter};
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};

/// Receives and deserializes a message with a big-endian 4-byte length header.
///
/// `buf` will be filled with the message data excluding the 4-byte length
/// header.
/// Deserialization uses bincode 2's standard configuration with serde and
/// permits trailing bytes after the decoded value.
///
/// # Errors
///
/// Returns an error if the message could not be read or deserialized.
pub async fn recv<'b, T>(recv: &mut RecvStream, buf: &'b mut Vec<u8>) -> io::Result<T>
where
    T: Deserialize<'b>,
{
    recv_raw(recv, buf).await?;
    bincode::serde::borrow_decode_from_slice(buf, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        .map(|(value, _)| value)
}

/// Receives a sequence of bytes with a big-endian 4-byte length header.
///
/// `buf` will be filled with the message data excluding the 4-byte length
/// header.
///
/// # Errors
///
/// Returns an error if the message could not be read.
pub async fn recv_raw(recv: &mut RecvStream, buf: &mut Vec<u8>) -> io::Result<()> {
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
    bincode::serde::encode_into_std_write(msg, buf, bincode::config::standard())
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
    use serde::{Deserialize, Serialize};

    use crate::test::{TOKEN, channel};

    #[tokio::test]
    async fn send_and_recv() {
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
        assert_eq!(buf, [] as [u8; 0]);
        let received = super::recv::<&str>(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(received, "hello");
    }

    #[tokio::test]
    async fn send_string() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let expected_payload: &[u8] = &[0x05, b'h', b'e', b'l', b'l', b'o'];

        let mut buf = Vec::new();
        super::send(&mut channel.server.send, &mut buf, "hello")
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected_payload);
    }

    #[tokio::test]
    async fn recv_string() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let payload: &[u8] = &[0x05, b'w', b'o', b'r', b'l', b'd'];
        super::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let result: &str = super::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(result, "world");
    }

    #[tokio::test]
    async fn send_u32_small() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let expected_payload: &[u8] = &[0x2A];

        let mut buf = Vec::new();
        super::send(&mut channel.server.send, &mut buf, 42u32)
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected_payload);
    }

    #[tokio::test]
    async fn send_u32_large() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let expected_payload: &[u8] = &[0xFB, 0xE8, 0x03];

        let mut buf = Vec::new();
        super::send(&mut channel.server.send, &mut buf, 1000u32)
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected_payload);
    }

    #[tokio::test]
    async fn recv_u32() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let payload: &[u8] = &[0xFC, 0xEF, 0xBE, 0xAD, 0xDE];
        super::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let result: u32 = super::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(result, 0xDEAD_BEEF);
    }

    #[tokio::test]
    async fn send_i32_negative() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let expected_payload: &[u8] = &[0x01];

        let mut buf = Vec::new();
        super::send(&mut channel.server.send, &mut buf, -1i32)
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected_payload);
    }

    #[tokio::test]
    async fn recv_i32_negative() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let payload: &[u8] = &[0xC7];
        super::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let result: i32 = super::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(result, -100);
    }

    #[tokio::test]
    async fn send_result_ok() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let expected_payload: &[u8] = &[0x00, 0x02, b'h', b'i'];

        let mut buf = Vec::new();
        let value: Result<&str, &str> = Ok("hi");
        super::send(&mut channel.server.send, &mut buf, value)
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected_payload);
    }

    #[tokio::test]
    async fn send_result_err() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let expected_payload: &[u8] = &[0x01, 0x04, b'f', b'a', b'i', b'l'];

        let mut buf = Vec::new();
        let value: Result<(), String> = Err(String::from("fail"));
        super::send(&mut channel.server.send, &mut buf, value)
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected_payload);
    }

    #[tokio::test]
    async fn recv_result() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let ok_payload: &[u8] = &[0x00, 0x04, b't', b'e', b's', b't'];
        super::send_raw(&mut channel.server.send, ok_payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let result: Result<&str, &str> = super::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(result, Ok("test"));

        let err_payload: &[u8] = &[0x01, 0x02, b'n', b'o'];
        super::send_raw(&mut channel.server.send, err_payload)
            .await
            .unwrap();

        let result: Result<(), &str> = super::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(result, Err("no"));
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestStruct {
        id: u32,
        name: String,
    }

    #[tokio::test]
    async fn send_struct() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let expected_payload: &[u8] = &[0x63, 0x02, b'a', b'b'];

        let mut buf = Vec::new();
        let value = TestStruct {
            id: 99,
            name: String::from("ab"),
        };
        super::send(&mut channel.server.send, &mut buf, &value)
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected_payload);
    }

    #[tokio::test]
    async fn recv_struct() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let payload: &[u8] = &[0x7B, 0x04, b't', b'e', b's', b't'];
        super::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let result: TestStruct = super::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(
            result,
            TestStruct {
                id: 123,
                name: String::from("test")
            }
        );
    }

    #[tokio::test]
    async fn send_vec() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let expected_payload: &[u8] = &[0x03, 0x64, 0xC8, 0xFB, 0x2C, 0x01];

        let mut buf = Vec::new();
        let value: Vec<u32> = vec![100, 200, 300];
        super::send(&mut channel.server.send, &mut buf, &value)
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected_payload);
    }

    #[tokio::test]
    async fn recv_vec() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let payload: &[u8] = &[0x02, 0x01, b'a', 0x02, b'b', b'b'];
        super::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let result: Vec<String> = super::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(result, vec!["a", "bb"]);
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    enum TestEnum {
        Empty,
        WithValue(u32),
    }

    #[tokio::test]
    async fn send_enum() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let mut buf = Vec::new();
        super::send(&mut channel.server.send, &mut buf, TestEnum::Empty)
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, &[0x00]);

        super::send(&mut channel.server.send, &mut buf, TestEnum::WithValue(500))
            .await
            .unwrap();
        super::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, &[0x01, 0xFB, 0xF4, 0x01]);
    }

    #[tokio::test]
    async fn recv_enum() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let payload: &[u8] = &[0x01, 0xFB, 0xE8, 0x03];
        super::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let result: TestEnum = super::recv(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(result, TestEnum::WithValue(1000));
    }
}
