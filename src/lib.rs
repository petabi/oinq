use std::{mem, num::TryFromIntError};

use bincode::Options;
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecvError {
    #[error("failed deserializing message")]
    DeserializationFailure(#[from] bincode::Error),
    #[error("failed to read from a stream")]
    ReadError(#[from] quinn::ReadExactError),
}

/// Receives a message with a big-endian 4-byte length header.
///
/// # Errors
///
/// * `RecvError::DeserializationFailure`: if the message could not be
///   deserialized
/// * `RecvError::ReadError`: if the message could not be read
pub async fn recv_frame<'b, T>(recv: &mut RecvStream, buf: &'b mut Vec<u8>) -> Result<T, RecvError>
where
    T: Deserialize<'b>,
{
    let mut len_buf = [0; mem::size_of::<u32>()];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    buf.resize(len, 0);
    recv.read_exact(buf.as_mut_slice()).await?;
    Ok(bincode::DefaultOptions::new().deserialize(buf)?)
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error("failed serializing message")]
    SerializationFailure(#[from] bincode::Error),
    #[error("message is too large")]
    MessageTooLarge(#[from] TryFromIntError),
    #[error("failed to write to a stream")]
    WriteError(#[from] quinn::WriteError),
}

/// Sends a message as a stream of bytes with a big-endian 4-byte length header.
///
/// # Errors
///
/// * `SendError::SerializationFailure`: if the message could not be serialized
/// * `SendError::MessageTooLarge`: if the message is too large
/// * `SendError::WriteError`: if the message could not be written
pub async fn send_frame<T>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    msg: T,
) -> Result<(), SendError>
where
    T: Serialize,
{
    buf.resize(mem::size_of::<u32>(), 0);
    bincode::DefaultOptions::new().serialize_into(&mut *buf, &msg)?;
    let len = u32::try_from(buf.len() - 4)?;
    buf[..mem::size_of::<u32>()].clone_from_slice(&len.to_be_bytes());
    send.write_all(buf).await?;
    buf.clear();
    Ok(())
}
