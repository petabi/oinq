//! Low-level communication primitives for QUIC applications and RPC-like protocols.
//!
//! [`transport`] provides owned endpoints, connections, and Tokio streams with
//! caller-supplied rustls policy and no Quinn types in its public signatures.
//! See [`frame`] for bounded framing over Tokio I/O, including Quinn streams,
//! and a usage example. [`message`] provides request and response helpers, and
//! [`request`] defines request codes and argument parsing.

pub mod frame;
pub mod message;
pub mod request;
#[cfg(test)]
mod test;

pub mod transport;
pub use transport::{
    ApplicationCode, CloseInfo, Connection, Endpoint, Incoming, RecvStream, SendStream,
    TransportError, TransportErrorKind, TransportOptions,
};
