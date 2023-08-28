//! Functions and errors for handling messages.

use crate::{
    frame::{self, RecvError, SendError},
    RequestCode,
};
use bincode::Options;
use quinn::{Connection, ConnectionError, RecvStream, SendStream};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::{
    fmt, mem,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};
use thiserror::Error;

/// Receives a message as a stream of bytes with a big-endian 4-byte length
/// header.
///
/// `buf` will be filled with the message.
///
/// # Errors
///
/// * `RecvError::DeserializationFailure` if the message could not be
///   deserialized
/// * `RecvError::ReadError` if the message could not be read
///
/// # Panics
///
/// * panic if it failed to convert 4 byte data to u32 value
pub async fn recv_request_raw<'b>(
    recv: &mut RecvStream,
    buf: &'b mut Vec<u8>,
) -> Result<(u32, &'b [u8]), RecvError> {
    frame::recv_raw(recv, buf).await?;
    if buf.len() < mem::size_of::<u32>() {
        return Err(RecvError::DeserializationFailure(Box::new(
            bincode::ErrorKind::SizeLimit,
        )));
    }
    let code = u32::from_le_bytes(buf[..mem::size_of::<u32>()].try_into().expect("4 bytes"));
    Ok((code, buf[mem::size_of::<u32>()..].as_ref()))
}

/// The error type for a handshake failure.
#[derive(Debug, Error)]
pub enum HandshakeError {
    #[error("connection closed by peer")]
    ConnectionClosed,
    #[error("connection lost")]
    ConnectionLost(#[from] ConnectionError),
    #[error("cannot receive a message")]
    ReadError(#[from] quinn::ReadError),
    #[error("cannot send a message")]
    WriteError(#[from] quinn::WriteError),
    #[error("cannot serialize a message")]
    SerializationFailure(#[from] bincode::Error),
    #[error("arguments are too long")]
    MessageTooLarge,
    #[error("invalid message")]
    InvalidMessage,
    #[error("protocol version {0} is not supported; version {1} is required")]
    IncompatibleProtocol(String, String),
}

impl From<SendError> for HandshakeError {
    fn from(e: SendError) -> Self {
        match e {
            SendError::SerializationFailure(e) => HandshakeError::SerializationFailure(e),
            SendError::MessageTooLarge(_) => HandshakeError::MessageTooLarge,
            SendError::WriteError(e) => HandshakeError::WriteError(e),
        }
    }
}

/// Properties of an agent.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentInfo {
    pub app_name: String,
    pub version: String,
    pub protocol_version: String,
    pub addr: SocketAddr,
}

impl std::fmt::Display for AgentInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}, {}", self.app_name, self.addr)
    }
}

/// Sends a handshake request and processes the response.
///
/// # Errors
///
/// Returns `HandshakeError` if the handshake failed.
pub async fn client_handshake(
    conn: &Connection,
    app_name: &str,
    app_version: &str,
    protocol_version: &str,
) -> Result<(SendStream, RecvStream), HandshakeError> {
    // A placeholder for the address of this agent. Will be replaced by the
    // server.
    //
    // TODO: This is unnecessary in handshake, and thus should be removed in the
    // future.
    let addr = if conn.remote_address().is_ipv6() {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
    } else {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
    };

    let agent_info = AgentInfo {
        app_name: app_name.to_string(),
        version: app_version.to_string(),
        protocol_version: protocol_version.to_string(),
        addr,
    };

    let (mut send, mut recv) = conn.open_bi().await?;
    let mut buf = Vec::new();
    if let Err(e) = frame::send(&mut send, &mut buf, &agent_info).await {
        match e {
            SendError::SerializationFailure(e) => {
                return Err(HandshakeError::SerializationFailure(e))
            }
            SendError::MessageTooLarge(_) => return Err(HandshakeError::MessageTooLarge),
            SendError::WriteError(e) => return Err(HandshakeError::WriteError(e)),
        }
    }

    match frame::recv_raw(&mut recv, &mut buf).await {
        Ok(_) => {}
        Err(quinn::ReadExactError::FinishedEarly) => {
            return Err(HandshakeError::ConnectionClosed);
        }
        Err(quinn::ReadExactError::ReadError(e)) => {
            return Err(HandshakeError::ReadError(e));
        }
    }
    let de = bincode::DefaultOptions::new();
    de.deserialize::<Result<&str, &str>>(&buf)
        .map_err(|_| HandshakeError::InvalidMessage)?
        .map_err(|e| {
            HandshakeError::IncompatibleProtocol(protocol_version.to_string(), e.to_string())
        })?;

    Ok((send, recv))
}

/// Processes a handshake message and sends a response.
///
/// # Errors
///
/// Returns `HandshakeError` if the handshake failed.
///
/// # Panics
///
/// * panic if it failed to parse version requirement string.
pub async fn server_handshake(
    conn: &Connection,
    addr: SocketAddr,
    version_req: &str,
    highest_protocol_version: &str,
) -> Result<AgentInfo, HandshakeError> {
    let (mut send, mut recv) = conn
        .accept_bi()
        .await
        .map_err(HandshakeError::ConnectionLost)?;
    let mut buf = Vec::new();
    let mut agent_info = frame::recv::<AgentInfo>(&mut recv, &mut buf)
        .await
        .map_err(|_| HandshakeError::InvalidMessage)?;
    agent_info.addr = addr;
    let version_req = VersionReq::parse(version_req).expect("valid version requirement");
    let protocol_version = Version::parse(&agent_info.protocol_version).map_err(|_| {
        HandshakeError::IncompatibleProtocol(
            agent_info.protocol_version.clone(),
            version_req.to_string(),
        )
    })?;
    if version_req.matches(&protocol_version) {
        let highest_protocol_version =
            Version::parse(highest_protocol_version).expect("valid semver");
        if protocol_version <= highest_protocol_version {
            send_ok(&mut send, &mut buf, highest_protocol_version.to_string())
                .await
                .map_err(HandshakeError::from)?;
            Ok(agent_info)
        } else {
            send_err(&mut send, &mut buf, &highest_protocol_version)
                .await
                .map_err(HandshakeError::from)?;
            send.finish().await.ok();
            Err(HandshakeError::IncompatibleProtocol(
                protocol_version.to_string(),
                version_req.to_string(),
            ))
        }
    } else {
        send_err(&mut send, &mut buf, version_req.to_string())
            .await
            .map_err(HandshakeError::from)?;
        send.finish().await.ok();
        Err(HandshakeError::IncompatibleProtocol(
            protocol_version.to_string(),
            version_req.to_string(),
        ))
    }
}

/// Sends a request with a big-endian 4-byte length header.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_request<C, B>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    code: C,
    body: B,
) -> Result<(), SendError>
where
    C: Into<u32>,
    B: Serialize,
{
    buf.clear();
    serialize_request(buf, code, body)?;
    frame::send_raw(send, buf).await?;
    buf.clear();
    Ok(())
}

/// Sends a request encapsulated in a `RequestCode::Forward` request, with a
/// big-endian 4-byte length header.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_forward_request<C, B>(
    send: &mut SendStream,
    mut buf: &mut Vec<u8>,
    dst: &str,
    code: C,
    body: B,
) -> Result<(), SendError>
where
    C: Into<u32>,
    B: Serialize,
{
    buf.clear();
    buf.extend_from_slice(&u32::from(RequestCode::Forward).to_le_bytes());
    let codec = bincode::DefaultOptions::new();
    codec.serialize_into(&mut buf, dst)?;
    let len = codec.serialized_size(&body)? + mem::size_of::<u32>() as u64;
    codec.serialize_into(&mut buf, &len)?;
    serialize_request(buf, code, body)?;
    frame::send_raw(send, buf).await?;
    buf.clear();
    Ok(())
}

/// Serializes the given request and appends it to `buf`.
///
/// # Errors
///
/// * `bincode::Error` if the message could not be serialized
fn serialize_request<C, B>(buf: &mut Vec<u8>, code: C, body: B) -> Result<(), bincode::Error>
where
    C: Into<u32>,
    B: Serialize,
{
    buf.extend_from_slice(&code.into().to_le_bytes());
    bincode::DefaultOptions::new().serialize_into(buf, &body)?;
    Ok(())
}

/// Sends an `Ok` response.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_ok<T: Serialize>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    body: T,
) -> Result<(), SendError> {
    frame::send(send, buf, Ok(body) as Result<T, &str>).await?;
    Ok(())
}

/// Sends an `Err` response.
///
/// `buf` will be cleared after the response is sent.
///
/// # Errors
///
/// * `SendError::SerializationFailure` if the message could not be serialized
/// * `SendError::WriteError` if the message could not be written
pub async fn send_err<E: fmt::Display>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    e: E,
) -> Result<(), SendError> {
    frame::send(send, buf, Err(format!("{e:#}")) as Result<(), String>).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::mem;

    use bincode::Options;

    use crate::test::{channel, TOKEN};
    use crate::{frame, RequestCode};

    #[tokio::test]
    async fn handshake() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        const APP_NAME: &str = "oinq";
        const APP_VERSION: &str = "1.0.0";
        const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

        let _lock = TOKEN.lock().await;
        let channel = channel().await;
        let (server, client) = (channel.server, channel.client);

        let handle = tokio::spawn(async move {
            super::client_handshake(&client.conn, APP_NAME, APP_VERSION, PROTOCOL_VERSION).await
        });

        let agent_info = super::server_handshake(
            &server.conn,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            PROTOCOL_VERSION,
            PROTOCOL_VERSION,
        )
        .await
        .unwrap();

        assert_eq!(agent_info.app_name, APP_NAME);
        assert_eq!(agent_info.version, APP_VERSION);
        assert_eq!(agent_info.protocol_version, PROTOCOL_VERSION);

        assert_eq!(
            agent_info.to_string(),
            format!("{}, {}", agent_info.app_name, agent_info.addr)
        );
        let res = tokio::join!(handle).0.unwrap();
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn handshake_version_incompatible_err() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        const APP_NAME: &str = "oinq";
        const APP_VERSION: &str = "1.0.0";
        const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

        let _lock = TOKEN.lock().await;
        let channel = channel().await;
        let (server, client) = (channel.server, channel.client);

        let handle = tokio::spawn(async move {
            super::client_handshake(&client.conn, APP_NAME, APP_VERSION, PROTOCOL_VERSION).await
        });

        let res = super::server_handshake(
            &server.conn,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            &format!("<{PROTOCOL_VERSION}"),
            PROTOCOL_VERSION,
        )
        .await;

        assert!(res.is_err());

        let res = tokio::join!(handle).0.unwrap();
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn handshake_incompatible_err() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        const APP_NAME: &str = "oinq";
        const APP_VERSION: &str = "1.0.0";
        const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

        let version_req = semver::VersionReq::parse(&format!(">={PROTOCOL_VERSION}")).unwrap();
        let mut highest_version = semver::Version::parse(PROTOCOL_VERSION).unwrap();
        highest_version.patch += 1;
        let mut protocol_version = highest_version.clone();
        protocol_version.minor += 1;

        let _lock = TOKEN.lock().await;
        let channel = channel().await;
        let (server, client) = (channel.server, channel.client);

        let handle = tokio::spawn(async move {
            super::client_handshake(
                &client.conn,
                APP_NAME,
                APP_VERSION,
                &protocol_version.to_string(),
            )
            .await
        });

        let res = super::server_handshake(
            &server.conn,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            &version_req.to_string(),
            &highest_version.to_string(),
        )
        .await;

        assert!(res.is_err());

        let res = tokio::join!(handle).0.unwrap();
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn send_and_recv() {
        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let mut buf = Vec::new();
        super::send_request(
            &mut channel.server.send,
            &mut buf,
            RequestCode::ReloadTi,
            (),
        )
        .await
        .unwrap();
        assert!(buf.is_empty());
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, u32::from(RequestCode::ReloadTi));
        assert!(body.is_empty());

        buf.clear();
        super::send_forward_request(
            &mut channel.server.send,
            &mut buf,
            "agent@host",
            RequestCode::ReloadTi,
            (),
        )
        .await
        .unwrap();
        let (code, body) = super::recv_request_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(code, u32::from(RequestCode::Forward));
        let (dst, msg) = bincode::DefaultOptions::new()
            .deserialize::<(String, &[u8])>(body)
            .unwrap();
        assert_eq!(dst, "agent@host");
        assert_eq!(msg.len(), mem::size_of::<u32>());
        let code = u32::from_le_bytes(msg[..mem::size_of::<u32>()].try_into().expect("4 bytes"));
        assert_eq!(RequestCode::from(code), RequestCode::ReloadTi);
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
}
