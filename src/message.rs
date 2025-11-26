//! Functions and errors for handling messages.

use std::{fmt, io, mem};

use quinn::{RecvStream, SendStream};
use serde::Serialize;

use crate::frame;

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
    buf.clear();
    buf.extend_from_slice(&code.into().to_le_bytes());
    bincode::serde::encode_into_std_write(body, buf, bincode::config::standard())
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
    use crate::test::{TOKEN, channel};

    #[tokio::test]
    async fn send_and_recv() {
        const CODE: u32 = 0x1234;

        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

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

    use serde::{Deserialize, Serialize};

    use crate::request;

    #[tokio::test]
    async fn send_request_empty_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 0x1234 in LE: [0x34, 0x12, 0x00, 0x00]
        // body = (): empty
        let expected: &[u8] = &[0x34, 0x12, 0x00, 0x00];

        let mut buf = Vec::new();
        super::send_request(&mut channel.server.send, &mut buf, 0x1234u32, ())
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected);
    }

    #[tokio::test]
    async fn recv_request_empty_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 0xABCD in LE, empty body
        let payload: &[u8] = &[0xCD, 0xAB, 0x00, 0x00];
        frame::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, 0xABCD);
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn send_request_string_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 1 in LE: [0x01, 0x00, 0x00, 0x00]
        // body = "hi": [0x02, 'h', 'i']
        let expected: &[u8] = &[0x01, 0x00, 0x00, 0x00, 0x02, b'h', b'i'];

        let mut buf = Vec::new();
        super::send_request(&mut channel.server.send, &mut buf, 1u32, "hi")
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected);
    }

    #[tokio::test]
    async fn recv_request_string_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 99 in LE, body = "test"
        let payload: &[u8] = &[0x63, 0x00, 0x00, 0x00, 0x04, b't', b'e', b's', b't'];
        frame::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, 99);
        let parsed: &str = request::parse_args(body).unwrap();
        assert_eq!(parsed, "test");
    }

    #[tokio::test]
    async fn send_request_u32_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 0 in LE: [0x00, 0x00, 0x00, 0x00]
        // body = 1000u32: [0xFB, 0xE8, 0x03] (varint)
        let expected: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0xFB, 0xE8, 0x03];

        let mut buf = Vec::new();
        super::send_request(&mut channel.server.send, &mut buf, 0u32, 1000u32)
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected);
    }

    #[tokio::test]
    async fn recv_request_u32_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 42 in LE, body = 0xDEADBEEF
        let payload: &[u8] = &[0x2A, 0x00, 0x00, 0x00, 0xFC, 0xEF, 0xBE, 0xAD, 0xDE];
        frame::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, 42);
        let parsed: u32 = request::parse_args(body).unwrap();
        assert_eq!(parsed, 0xDEAD_BEEF);
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestStruct {
        id: u32,
        name: String,
    }

    #[tokio::test]
    async fn send_request_struct_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 100 in LE: [0x64, 0x00, 0x00, 0x00]
        // body = TestStruct { id: 1, name: "a" }: [0x01, 0x01, 'a']
        let expected: &[u8] = &[0x64, 0x00, 0x00, 0x00, 0x01, 0x01, b'a'];

        let mut buf = Vec::new();
        let value = TestStruct {
            id: 1,
            name: String::from("a"),
        };
        super::send_request(&mut channel.server.send, &mut buf, 100u32, &value)
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected);
    }

    #[tokio::test]
    async fn recv_request_struct_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 200 in LE, body = TestStruct { id: 99, name: "xy" }
        let payload: &[u8] = &[0xC8, 0x00, 0x00, 0x00, 0x63, 0x02, b'x', b'y'];
        frame::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, 200);
        let parsed: TestStruct = request::parse_args(body).unwrap();
        assert_eq!(
            parsed,
            TestStruct {
                id: 99,
                name: String::from("xy")
            }
        );
    }

    #[tokio::test]
    async fn send_request_vec_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 5 in LE: [0x05, 0x00, 0x00, 0x00]
        // body = vec![1u8, 2, 3]: [0x03, 0x01, 0x02, 0x03]
        let expected: &[u8] = &[0x05, 0x00, 0x00, 0x00, 0x03, 0x01, 0x02, 0x03];

        let mut buf = Vec::new();
        super::send_request(&mut channel.server.send, &mut buf, 5u32, vec![1u8, 2, 3])
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected);
    }

    #[tokio::test]
    async fn recv_request_vec_body() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 10 in LE, body = vec!["a", "bb"]
        let payload: &[u8] = &[0x0A, 0x00, 0x00, 0x00, 0x02, 0x01, b'a', 0x02, b'b', b'b'];
        frame::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, 10);
        let parsed: Vec<String> = request::parse_args(body).unwrap();
        assert_eq!(parsed, vec!["a", "bb"]);
    }

    #[tokio::test]
    async fn send_request_large_code() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 0xDEADBEEF in LE: [0xEF, 0xBE, 0xAD, 0xDE]
        // body = (): empty
        let expected: &[u8] = &[0xEF, 0xBE, 0xAD, 0xDE];

        let mut buf = Vec::new();
        super::send_request(&mut channel.server.send, &mut buf, 0xDEAD_BEEFu32, ())
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, expected);
    }

    #[tokio::test]
    async fn recv_request_large_code() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        // code = 0xFFFFFFFF in LE
        let payload: &[u8] = &[0xFF, 0xFF, 0xFF, 0xFF];
        frame::send_raw(&mut channel.server.send, payload)
            .await
            .unwrap();

        let mut buf = Vec::new();
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, 0xFFFF_FFFF);
        assert!(body.is_empty());
    }
}
