use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures::FutureExt;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

use super::{FrameError, FrameLimit, FramePart, FramedReader, FramedWriter};

fn limit(n: usize) -> FrameLimit {
    FrameLimit::new(n).unwrap()
}

#[tokio::test]
async fn limits_eof_and_recycling() {
    let mut reader = FramedReader::new(&b"\0\0\0\0\0\0\0\x03abc"[..], limit(3));
    assert_eq!(reader.read().await.unwrap().unwrap().as_bytes(), []);
    let frame = reader.read().await.unwrap().unwrap();
    assert_eq!(frame.as_bytes(), b"abc");
    reader.recycle(frame.into_bytes()).unwrap();
    assert!(reader.read().await.unwrap().is_none());
    assert!(reader.read().await.unwrap().is_none());

    let mut reader = FramedReader::new(&b"\xff\xff\xff\xff"[..], limit(3));
    assert!(matches!(
        reader.read().await,
        Err(FrameError::TooLarge { .. })
    ));
    assert_eq!(reader.payload.capacity(), 0);
    assert!(matches!(reader.read().await, Err(FrameError::Poisoned)));
    let mut writer = FramedWriter::new(Vec::new(), limit(0));
    writer.start_raw(b"").unwrap();
    writer.flush().await.unwrap();
    assert_eq!(writer.into_inner(), [0; 4]);
    if usize::BITS > 32 {
        assert!(FrameLimit::new(usize::MAX).is_err());
    }
}

#[tokio::test]
async fn recycled_buffers_preserve_initialized_bytes_until_overwritten() {
    let (mut peer, io) = tokio::io::duplex(32);
    let mut reader = FramedReader::new(io, limit(8));
    let mut buffer = Vec::with_capacity(8);
    buffer.extend_from_slice(b"seed");
    let allocation = buffer.as_ptr();

    for length in [4, 2, 6, 0, 3] {
        let mut initialized = buffer.clone();
        initialized.resize(length, 0);
        reader.recycle(buffer).unwrap();
        peer.write_all(&u32::try_from(length).unwrap().to_be_bytes())
            .await
            .unwrap();

        if length > 0 {
            // Pause after the header: existing bytes must survive resizing,
            // while any newly exposed bytes are safely initialized.
            assert!(reader.read().now_or_never().is_none());
            assert_eq!(reader.payload, initialized);
        }

        let payload = vec![b'x'; length];
        peer.write_all(&payload).await.unwrap();
        let frame = reader.read().await.unwrap().unwrap();
        assert_eq!(frame.as_bytes(), payload);
        buffer = frame.into_bytes();
        assert_eq!(buffer.as_ptr(), allocation);
    }
    peer.shutdown().await.unwrap();
    assert!(reader.read().await.unwrap().is_none());
}

#[tokio::test]
async fn all_truncation_offsets() {
    let wire = b"\0\0\0\x03abc";
    for end in 1..wire.len() {
        let mut reader = FramedReader::new(&wire[..end], limit(3));
        let (part, received, expected) = if end < 4 {
            (FramePart::Header, end, 4)
        } else {
            (FramePart::Payload, end - 4, 3)
        };
        assert!(
            matches!(reader.read().await, Err(FrameError::Truncated {part: p, received: r, expected: e}) if (p,r,e) == (part,received,expected))
        );
        assert!(matches!(reader.read().await, Err(FrameError::Poisoned)));
    }
}

#[tokio::test]
async fn cancelled_reads_resume_at_every_offset() {
    let wire = b"\0\0\0\x03abc";
    for offset in 0..wire.len() {
        let (mut peer, io) = tokio::io::duplex(32);
        let mut reader = FramedReader::new(io, limit(3));
        peer.write_all(&wire[..offset]).await.unwrap();
        assert!(reader.read().now_or_never().is_none());
        if offset > 0 {
            assert!(matches!(
                reader.recycle(Vec::new()),
                Err(FrameError::Pending)
            ));
        }
        peer.write_all(&wire[offset..]).await.unwrap();
        peer.shutdown().await.unwrap();
        assert_eq!(reader.read().await.unwrap().unwrap().as_bytes(), b"abc");
        assert!(reader.read().await.unwrap().is_none());
    }
}

#[tokio::test]
async fn cancelled_writes_resume_at_every_offset() {
    let wire = b"\0\0\0\x03abc";
    for offset in 1..wire.len() {
        let (io, mut peer) = tokio::io::duplex(offset);
        let mut writer = FramedWriter::new(io, limit(3));
        writer.start_raw(b"abc").unwrap();
        assert!(writer.flush().now_or_never().is_none());
        assert!(writer.is_pending());
        assert!(matches!(writer.start_raw(b"xyz"), Err(FrameError::Pending)));
        assert!(matches!(writer.start(1u8), Err(FrameError::Pending)));
        let mut actual = vec![0; wire.len()];
        peer.read_exact(&mut actual[..offset]).await.unwrap();
        let (sent, received) = tokio::join!(writer.flush(), peer.read_exact(&mut actual[offset..]));
        sent.unwrap();
        received.unwrap();
        assert_eq!(actual, wire);
        assert!(!writer.is_pending());
    }
}

struct Bad;
impl Serialize for Bad {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("failure"))
    }
}

#[tokio::test]
async fn capped_encoding_and_modes() {
    let mut writer = FramedWriter::new(Vec::new(), limit(3));
    assert!(matches!(
        writer.start("abc"),
        Err(FrameError::TooLarge { .. })
    ));
    assert_eq!(writer.io, [] as [u8; 0]);
    assert!(matches!(
        writer.start_raw(b"abcd"),
        Err(FrameError::TooLarge { .. })
    ));
    writer.start(1000u32).unwrap();
    writer.flush().await.unwrap();
    assert_eq!(writer.into_inner(), [0, 0, 0, 3, 251, 232, 3]);

    let mut writer = FramedWriter::new(Vec::new(), limit(16));
    assert!(matches!(writer.start(Bad), Err(FrameError::Encode(_))));
    assert_eq!(writer.io, [] as [u8; 0]);
    writer.start_raw(&[42, 99]).unwrap();
    writer.flush().await.unwrap();
    let wire = writer.into_inner();
    let mut reader = FramedReader::new(wire.as_slice(), limit(16));
    let frame = reader.read().await.unwrap().unwrap();
    assert_eq!(frame.decode::<u32>().unwrap(), 42);
    assert!(matches!(
        frame.decode_strict::<u32>(),
        Err(FrameError::TrailingBytes { .. })
    ));
}

#[tokio::test]
async fn nested_collections_cannot_escape_budget() {
    // A small outer vector, then a malicious nested collection count.
    let payload = [1, 253, 255, 255, 255, 255, 255, 255, 255, 127];
    let mut writer = FramedWriter::new(Vec::new(), limit(64));
    writer.start_raw(&payload).unwrap();
    writer.flush().await.unwrap();
    let bytes = writer.into_inner();
    let frame = FramedReader::new(bytes.as_slice(), limit(64))
        .read()
        .await
        .unwrap()
        .unwrap();
    assert!(frame.decode::<Vec<Vec<u8>>>().is_err());
    assert!(frame.decode::<Vec<Vec<()>>>().is_err());

    let mut writer = FramedWriter::new(Vec::new(), limit(128));
    writer.start(vec![vec![1u8, 2], vec![3]]).unwrap();
    writer.flush().await.unwrap();
    let bytes = writer.into_inner();
    let frame = FramedReader::new(bytes.as_slice(), limit(128))
        .read()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        frame.decode_strict::<Vec<Vec<u8>>>().unwrap(),
        [vec![1, 2], vec![3]]
    );
}

struct Fault {
    bytes: &'static [u8],
    remaining: usize,
    kind: io::ErrorKind,
}
impl AsyncRead for Fault {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.remaining == 0 {
            return Poll::Ready(Err(io::Error::new(self.kind, "transport failure")));
        }
        let n = self.remaining.min(buf.remaining()).min(self.bytes.len());
        buf.put_slice(&self.bytes[..n]);
        self.bytes = &self.bytes[n..];
        self.remaining -= n;
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for Fault {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.remaining == 0 {
            return Poll::Ready(Err(io::Error::new(self.kind, "transport failure")));
        }
        let n = self.remaining.min(bytes.len());
        self.remaining -= n;
        Poll::Ready(Ok(n))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn transport_errors_preserve_kind_and_poison() {
    for kind in [
        io::ErrorKind::ConnectionReset,
        io::ErrorKind::BrokenPipe,
        io::ErrorKind::Other,
    ] {
        for offset in [0, 2, 5] {
            let fault = || Fault {
                bytes: b"\0\0\0\x03abc",
                remaining: offset,
                kind,
            };
            let mut reader = FramedReader::new(fault(), limit(3));
            assert!(
                matches!(reader.read().await, Err(FrameError::Io(e)) if e.kind() == kind && e.to_string() == "transport failure")
            );
            assert!(matches!(reader.read().await, Err(FrameError::Poisoned)));
            let mut writer = FramedWriter::new(fault(), limit(3));
            writer.start_raw(b"abc").unwrap();
            assert!(matches!(writer.flush().await, Err(FrameError::Io(e)) if e.kind() == kind));
            assert!(matches!(writer.flush().await, Err(FrameError::Poisoned)));
            assert!(matches!(writer.start_raw(b""), Err(FrameError::Poisoned)));
        }
    }
}

#[tokio::test]
async fn quinn_tokio_io_and_legacy_bytes() {
    let _lock = crate::test::TOKEN.lock().await;
    let channel = crate::test::channel().await;
    let mut writer = FramedWriter::new(channel.server.send, limit(32));
    let mut reader = FramedReader::new(channel.client.recv, limit(32));
    writer.start("hello").unwrap();
    writer.flush().await.unwrap();
    assert_eq!(
        reader
            .read()
            .await
            .unwrap()
            .unwrap()
            .decode_strict::<&str>()
            .unwrap(),
        "hello"
    );
    let mut send = writer.into_inner();
    send.reset(42u32.into()).unwrap();
    assert!(
        matches!(reader.read().await, Err(FrameError::Io(e)) if e.get_ref().and_then(|e| e.downcast_ref::<quinn::ReadError>()).is_some_and(|e| matches!(e, quinn::ReadError::Reset(code) if *code == 42u32.into())))
    );
}

#[tokio::test]
async fn golden_legacy_frames_and_messages() {
    async fn assert_wire(recv: &mut quinn::RecvStream, expected: &[u8]) {
        let mut actual = vec![0; expected.len()];
        recv.read_exact(&mut actual).await.unwrap();
        assert_eq!(actual, expected);
    }
    #[derive(Serialize)]
    struct Body {
        id: u32,
        name: &'static str,
    }
    let _lock = crate::test::TOKEN.lock().await;
    let mut channel = crate::test::channel().await;
    let send = &mut channel.server.send;
    let recv = &mut channel.client.recv;
    let mut buffer = Vec::new();
    super::super::send_raw(send, b"").await.unwrap();
    assert_wire(recv, &[0, 0, 0, 0]).await;
    super::super::send_raw(send, b"raw").await.unwrap();
    assert_wire(recv, b"\0\0\0\x03raw").await;
    super::super::send(send, &mut buffer, 1000u32)
        .await
        .unwrap();
    assert_wire(recv, &[0, 0, 0, 3, 251, 232, 3]).await;
    super::super::send(send, &mut buffer, Body { id: 99, name: "ab" })
        .await
        .unwrap();
    assert_wire(recv, &[0, 0, 0, 4, 99, 2, b'a', b'b']).await;
    crate::message::send_request(send, &mut buffer, 0x1234_5678u32, 1000u32)
        .await
        .unwrap();
    assert_wire(recv, &[0, 0, 0, 7, 0x78, 0x56, 0x34, 0x12, 251, 232, 3]).await;
    crate::message::send_ok(send, &mut buffer, "hi")
        .await
        .unwrap();
    assert_wire(recv, b"\0\0\0\x04\0\x02hi").await;
    crate::message::send_err(send, &mut buffer, "fail")
        .await
        .unwrap();
    assert_wire(recv, b"\0\0\0\x06\x01\x04fail").await;
}

#[tokio::test]
async fn cancellation_during_underlying_flush_retains_pending_frame() {
    use std::{cell::Cell, rc::Rc};
    struct FlushGate {
        bytes: Vec<u8>,
        ready: Rc<Cell<bool>>,
    }
    impl AsyncWrite for FlushGate {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.bytes.extend_from_slice(bytes);
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            if self.ready.get() {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    let ready = Rc::new(Cell::new(false));
    let mut writer = FramedWriter::new(
        FlushGate {
            bytes: Vec::new(),
            ready: ready.clone(),
        },
        limit(1),
    );
    writer.start_raw(b"x").unwrap();
    assert!(writer.flush().now_or_never().is_none());
    assert!(matches!(writer.start_raw(b"y"), Err(FrameError::Pending)));
    ready.set(true);
    writer.flush().await.unwrap();
    assert_eq!(writer.into_inner().bytes, b"\0\0\0\x01x");
}
