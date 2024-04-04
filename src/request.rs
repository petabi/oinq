//! Helper functions for request handlers.

use crate::{frame, message};
use bincode::Options;
use quinn::SendStream;
use serde::{Deserialize, Serialize};

/// Parses the arguments of a request.
///
/// # Errors
///
/// Returns `frame::RecvError::DeserializationFailure`: if the arguments could
/// not be deserialized.
pub fn parse_args<'de, T: Deserialize<'de>>(args: &'de [u8]) -> Result<T, frame::RecvError> {
    bincode::DefaultOptions::new()
        .deserialize::<T>(args)
        .map_err(frame::RecvError::DeserializationFailure)
}

/// Sends a response to a request.
///
/// # Errors
///
/// * `SendError::MessageTooLarge` if `e` is too large to be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_response<T: Serialize>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    body: T,
) -> Result<(), frame::SendError> {
    match frame::send(send, buf, body).await {
        Ok(()) => Ok(()),
        Err(frame::SendError::WriteError(e)) => Err(frame::SendError::WriteError(e)),
        Err(e) => message::send_err(send, buf, e).await,
    }
}
