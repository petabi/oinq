//! Owned, bounded framing for Tokio I/O.
use std::{fmt, io};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum encoded payload length, excluding the four-byte header.
/// Protocol implementations choose their own limits. Zero is valid.
///
/// This limit also supplies the allocation budget for [`Frame::decode`] and
/// [`Frame::decode_strict`]. Allow for in-memory collection sizes as well as
/// encoded payload sizes when selecting it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameLimit(u32);

impl FrameLimit {
    /// Checks that a payload limit fits the wire's `u32` length field.
    ///
    /// # Errors
    /// Returns [`FrameError::InvalidLimit`] above `u32::MAX`.
    pub fn new(bytes: usize) -> Result<Self, FrameError> {
        u32::try_from(bytes)
            .map(Self)
            .map_err(|_| FrameError::InvalidLimit)
    }

    /// Returns the payload limit in bytes.
    #[must_use]
    pub fn get(self) -> usize {
        self.0 as usize
    }

    fn check(self, length: usize) -> Result<u32, FrameError> {
        if length > self.get() {
            return Err(FrameError::TooLarge {
                length,
                limit: self,
            });
        }
        u32::try_from(length).map_err(|_| FrameError::InvalidLimit)
    }
}

/// Which part of a frame ended prematurely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePart {
    /// The four-byte length header.
    Header,
    /// The declared encoded payload.
    Payload,
}

/// Framing, codec, or transport failure. Transport errors retain their source.
#[derive(Debug)]
pub enum FrameError {
    /// A configured limit cannot be represented on the wire.
    InvalidLimit,
    /// An incoming or outgoing encoded payload exceeds the configured limit.
    /// For capped serialization, `length` is a lower bound on the encoded size.
    TooLarge { length: usize, limit: FrameLimit },
    /// Clean transport EOF within a frame, with progress in the indicated part.
    Truncated {
        part: FramePart,
        received: usize,
        expected: usize,
    },
    /// An original transport error, preserved without converting it to EOF.
    Io(io::Error),
    /// Serialization failed before transmission.
    Encode(bincode::error::EncodeError),
    /// Deserialization failed or exceeded its allocation budget.
    Decode(bincode::error::DecodeError),
    /// Strict decoding left unconsumed payload bytes.
    TrailingBytes { consumed: usize, length: usize },
    /// A frame is in progress and cannot be replaced.
    Pending,
    /// An earlier I/O or framing error made further operations unsafe.
    Poisoned,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit => f.write_str("frame limit exceeds u32::MAX"),
            Self::TooLarge { length, limit } => {
                write!(f, "payload length {length} exceeds {}", limit.get())
            }
            Self::Truncated {
                part,
                received,
                expected,
            } => write!(f, "truncated {part:?}: {received} of {expected} bytes"),
            Self::Io(e) => write!(f, "frame I/O: {e}"),
            Self::Encode(e) => write!(f, "frame encoding: {e}"),
            Self::Decode(e) => write!(f, "frame decoding: {e}"),
            Self::TrailingBytes { consumed, length } => {
                write!(f, "decoded {consumed} of {length} bytes")
            }
            Self::Pending => f.write_str("a frame is already pending"),
            Self::Poisoned => f.write_str("framing alignment was lost"),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Encode(e) => Some(e),
            Self::Decode(e) => Some(e),
            _ => None,
        }
    }
}

/// An owned payload. Decoded values may borrow from this frame.
/// Return its buffer with [`Self::into_bytes`] and [`FramedReader::recycle`]
/// to reuse the allocation; subsequent reads never overwrite a live frame.
#[derive(Debug)]
pub struct Frame {
    bytes: Vec<u8>,
    limit: FrameLimit,
}

impl Frame {
    /// Returns the encoded payload without its header.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Transfers the payload allocation to the caller.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    /// Decodes a prefix using bincode 2 standard serde encoding.
    ///
    /// Trailing payload bytes are permitted. Use [`Self::decode_strict`] to
    /// require whole-frame decoding. The decoded value may borrow from this frame.
    ///
    /// The configured frame budget also bounds collection allocation claims.
    /// It charges collection elements and string/byte storage, including nested
    /// collections, and hides untrusted collection size hints.
    /// This conservative accounting includes in-memory element sizes and may
    /// reject collections whose memory representation exceeds their wire size.
    /// Custom `Deserialize` implementations remain responsible for their own allocations.
    /// Decode failures leave the reader aligned for subsequent frames.
    /// # Errors
    /// Returns a decoding error for invalid or over-budget data.
    pub fn decode<'a, T: Deserialize<'a>>(&'a self) -> Result<T, FrameError> {
        self.decode_inner().map(|(value, _)| value)
    }
    /// Decodes exactly one value and rejects trailing payload bytes.
    ///
    /// Uses the same encoding, borrowing rules, and allocation budget as
    /// [`Self::decode`]. Decode failures leave the reader aligned for subsequent frames.
    /// # Errors
    /// Returns a decoding error or [`FrameError::TrailingBytes`].
    pub fn decode_strict<'a, T: Deserialize<'a>>(&'a self) -> Result<T, FrameError> {
        let (value, consumed) = self.decode_inner()?;
        if consumed != self.bytes.len() {
            return Err(FrameError::TrailingBytes {
                consumed,
                length: self.bytes.len(),
            });
        }
        Ok(value)
    }
    fn decode_inner<'a, T: Deserialize<'a>>(&'a self) -> Result<(T, usize), FrameError> {
        super::decode::decode(&self.bytes, self.limit.get()).map_err(FrameError::Decode)
    }
}

/// Owns an I/O reader and retains partial header/payload offsets across cancellation.
/// Cancelling `read` preserves framing state; it does not abort an application exchange.
/// Errors poison the reader; clean EOF is repeatable. Dropping this object drops
/// its I/O object. Generic Tokio I/O alone cannot guarantee a QUIC reset.
#[derive(Debug)]
pub struct FramedReader<R> {
    io: R,
    limit: FrameLimit,
    header: [u8; 4],
    header_pos: usize,
    payload: Vec<u8>,
    payload_pos: usize,
    poisoned: bool,
    eof: bool,
}

impl<R> FramedReader<R> {
    /// Takes ownership of the I/O object with a required payload limit.
    pub fn new(io: R, limit: FrameLimit) -> Self {
        Self {
            io,
            limit,
            header: [0; 4],
            header_pos: 0,
            payload: Vec::new(),
            payload_pos: 0,
            poisoned: false,
            eof: false,
        }
    }
    /// Supplies a spare allocation for future frames. May be called only between frames.
    ///
    /// Pass a previous frame's [`Frame::into_bytes`] result to reuse its buffer.
    /// Without recycling, each successful read transfers its allocation to the
    /// returned frame and the next read allocates a new buffer.
    /// # Errors
    /// Returns `Pending` during a read, or `Poisoned` after an error.
    pub fn recycle(&mut self, buffer: Vec<u8>) -> Result<(), FrameError> {
        if self.poisoned {
            return Err(FrameError::Poisoned);
        }
        if self.header_pos != 0 {
            return Err(FrameError::Pending);
        }
        // Retain initialized bytes so the next resize only zeroes any growth.
        self.payload = buffer;
        Ok(())
    }
    /// Releases the I/O object. Any retained partial frame state is discarded.
    pub fn into_inner(self) -> R {
        self.io
    }
}

impl<R: AsyncRead + Unpin> FramedReader<R> {
    /// Reads a frame, or `None` only for EOF before any new header byte.
    ///
    /// Safe to cancel and call again on the retained reader: partial header and
    /// payload offsets are preserved. The returned frame owns its bytes and may
    /// outlive subsequent reads.
    /// # Errors
    /// EOF within a frame is `Truncated`; transport errors retain their `io::Error`.
    /// Oversized declarations fail before resizing the payload buffer. All these
    /// errors poison the reader to prevent restarting a partially consumed frame.
    pub async fn read(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.poisoned {
            return Err(FrameError::Poisoned);
        }
        if self.eof {
            return Ok(None);
        }
        while self.header_pos < 4 {
            match self.io.read(&mut self.header[self.header_pos..]).await {
                Ok(0) if self.header_pos == 0 => {
                    self.eof = true;
                    return Ok(None);
                }
                Ok(0) => {
                    self.poisoned = true;
                    return Err(FrameError::Truncated {
                        part: FramePart::Header,
                        received: self.header_pos,
                        expected: 4,
                    });
                }
                Ok(n) => self.header_pos += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    self.poisoned = true;
                    return Err(FrameError::Io(e));
                }
            }
        }
        let length = u32::from_be_bytes(self.header) as usize;
        if let Err(e) = self.limit.check(length) {
            self.poisoned = true;
            return Err(e);
        }
        self.payload.resize(length, 0);
        while self.payload_pos < length {
            match self.io.read(&mut self.payload[self.payload_pos..]).await {
                Ok(0) => {
                    self.poisoned = true;
                    return Err(FrameError::Truncated {
                        part: FramePart::Payload,
                        received: self.payload_pos,
                        expected: length,
                    });
                }
                Ok(n) => self.payload_pos += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    self.poisoned = true;
                    return Err(FrameError::Io(e));
                }
            }
        }
        self.header_pos = 0;
        self.payload_pos = 0;
        Ok(Some(Frame {
            bytes: std::mem::take(&mut self.payload),
            limit: self.limit,
        }))
    }
}

/// Owns an I/O writer and one reusable payload buffer.
/// Prepare with `start_raw` or `start`, then transmit with `flush`. Cancellation
/// retains the same pending bytes and offsets; resume with `flush`. A pending
/// frame cannot be replaced, even by an identical value. Cancellation does not
/// abort an application exchange. Dropping this object drops its I/O object;
/// generic Tokio I/O alone cannot promise a QUIC reset.
#[derive(Debug)]
pub struct FramedWriter<W> {
    io: W,
    limit: FrameLimit,
    payload: Vec<u8>,
    header: [u8; 4],
    header_pos: usize,
    payload_pos: usize,
    pending: bool,
    poisoned: bool,
}

impl<W> FramedWriter<W> {
    /// Takes ownership of the I/O object with a required payload limit.
    pub fn new(io: W, limit: FrameLimit) -> Self {
        Self {
            io,
            limit,
            payload: Vec::new(),
            header: [0; 4],
            header_pos: 0,
            payload_pos: 0,
            pending: false,
            poisoned: false,
        }
    }
    fn ready(&self) -> Result<(), FrameError> {
        if self.poisoned {
            Err(FrameError::Poisoned)
        } else if self.pending {
            Err(FrameError::Pending)
        } else {
            Ok(())
        }
    }
    /// Copies a raw payload into the retained buffer without transmitting bytes.
    /// # Errors
    /// Returns `TooLarge`, `Pending`, or `Poisoned` without writing a header.
    pub fn start_raw(&mut self, bytes: &[u8]) -> Result<(), FrameError> {
        self.ready()?;
        let length = self.limit.check(bytes.len())?;
        self.payload.clear();
        self.payload.extend_from_slice(bytes);
        self.header = length.to_be_bytes();
        self.pending = true;
        Ok(())
    }
    /// Serializes into a capped, reusable buffer without transmitting bytes.
    ///
    /// Uses bincode 2's standard configuration with serde.
    /// # Errors
    /// Serialization and size errors write no header and leave the writer reusable.
    /// Returns `Pending` if a prior frame still needs to be flushed.
    pub fn start<T: Serialize>(&mut self, value: T) -> Result<(), FrameError> {
        self.ready()?;
        self.payload.clear();
        let mut capped = Capped {
            bytes: &mut self.payload,
            limit: self.limit.get(),
            exceeded: false,
        };
        let result =
            bincode::serde::encode_into_std_write(value, &mut capped, bincode::config::standard());
        if capped.exceeded {
            return Err(FrameError::TooLarge {
                length: self.limit.get().saturating_add(1),
                limit: self.limit,
            });
        }
        result.map_err(FrameError::Encode)?;
        self.header = self.limit.check(self.payload.len())?.to_be_bytes();
        self.pending = true;
        Ok(())
    }
    /// Whether a prepared frame still needs transmission or flushing.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.pending
    }
    /// Releases the I/O object, discarding any pending frame state.
    pub fn into_inner(self) -> W {
        self.io
    }
}

impl<W: AsyncWrite + Unpin> FramedWriter<W> {
    /// Completes the pending frame and flushes the underlying I/O object.
    /// Safe to cancel and call again on the retained writer.
    /// # Errors
    /// I/O errors (including write zero) poison the writer. The original error
    /// is retained, and subsequent operations return `Poisoned`.
    pub async fn flush(&mut self) -> Result<(), FrameError> {
        if self.poisoned {
            return Err(FrameError::Poisoned);
        }
        let result = self.flush_inner().await;
        if result.is_err() {
            self.poisoned = true;
        }
        result.map_err(FrameError::Io)
    }
    async fn flush_inner(&mut self) -> io::Result<()> {
        if self.pending {
            while self.header_pos < 4 {
                match self.io.write(&self.header[self.header_pos..]).await {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => self.header_pos += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            while self.payload_pos < self.payload.len() {
                match self.io.write(&self.payload[self.payload_pos..]).await {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => self.payload_pos += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
        }
        self.io.flush().await?;
        self.pending = false;
        self.payload.clear();
        self.header_pos = 0;
        self.payload_pos = 0;
        Ok(())
    }
}

struct Capped<'a> {
    bytes: &'a mut Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl io::Write for Capped<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            self.exceeded = true;
            return Err(io::Error::other("frame limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "bounded_tests.rs"]
mod tests;
