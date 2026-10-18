//! Low-level communication primitives for QUIC applications and RPC-like protocols.
//!
//! See [`frame`] for bounded framing over Tokio I/O, including Quinn streams,
//! and a usage example. [`message`] provides request and response helpers, and
//! [`request`] defines request codes and argument parsing.

pub mod frame;
pub mod message;
pub mod request;
#[cfg(test)]
mod test;
