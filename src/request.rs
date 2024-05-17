//! Helper functions for request handlers.

use std::io;

use crate::{frame, message};
use bincode::Options;
use quinn::SendStream;
use serde::{Deserialize, Serialize};

/// Parses the arguments of a request.
///
/// # Errors
///
/// Returns an error if the arguments could not be deserialized.
pub fn parse_args<'de, T: Deserialize<'de>>(args: &'de [u8]) -> io::Result<T> {
    bincode::DefaultOptions::new()
        .deserialize::<T>(args)
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed deserializing message: {e}"),
            )
        })
}

/// Sends a response to a request.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidData`] if the message is too large, or
/// other errors if the message could not be written to the stream.
pub async fn send_response<T: Serialize>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    body: T,
) -> io::Result<()> {
    if let Err(e) = frame::send(send, buf, body).await {
        if e.kind() == io::ErrorKind::InvalidData {
            message::send_err(send, buf, e).await
        } else {
            Err(e)
        }
    } else {
        Ok(())
    }
}
