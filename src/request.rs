//! Request handlers.

use async_trait::async_trait;
use bincode::Options;
use num_enum::FromPrimitive;
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{frame, message, RequestCode};

/// CPU, memory, and disk usage.
#[derive(Debug, Deserialize, Serialize)]
pub struct ResourceUsage {
    /// The average CPU usage in percent.
    pub cpu_usage: f32,

    /// The RAM size in KB.
    pub total_memory: u64,

    /// The amount of used RAM in KB.
    pub used_memory: u64,

    /// The total disk space in bytes.
    pub total_disk_space: u64,

    /// The total disk space in bytes that is currently used.
    pub used_disk_space: u64,
}

/// The error type for handling a request.
#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("failed to receive request")]
    RecvError(#[from] frame::RecvError),
    #[error("failed to send response")]
    SendError(#[from] frame::SendError),
}

/// A request handler that can handle a request to an agent.
#[async_trait]
pub trait Handler: Send {
    async fn dns_start(&mut self) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn dns_stop(&mut self) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn forward(&mut self, _target: &str, _msg: &[u8]) -> Result<Vec<u8>, String> {
        return Err("not supported".to_string());
    }

    /// Reboots the system
    async fn reboot(&mut self) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn reload_config(&mut self) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn reload_ti(&mut self, _version: &str) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    /// Returns the hostname and the cpu, memory, and disk usage.
    async fn resource_usage(&mut self) -> Result<(String, ResourceUsage), String> {
        return Err("not supported".to_string());
    }

    async fn tor_exit_node_list(&mut self, _nodes: &[&str]) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn trusted_domain_list(&mut self, _domains: &[&str]) -> Result<(), String> {
        return Err("not supported".to_string());
    }
}

/// Handles requests to an agent.
///
/// # Errors
///
/// * `HandlerError::RecvError` if the request could not be received
/// * `HandlerError::SendError` if the response could not be sent
pub async fn handle<H: Handler>(
    handler: &mut H,
    send: &mut SendStream,
    recv: &mut RecvStream,
) -> Result<(), HandlerError> {
    let mut buf = Vec::new();
    let codec = bincode::DefaultOptions::new();
    loop {
        let (code, body) = match message::recv_request_raw(recv, &mut buf).await {
            Ok(res) => res,
            Err(frame::RecvError::ReadError(quinn::ReadExactError::FinishedEarly)) => {
                break;
            }
            Err(e) => {
                return Err(e.into());
            }
        };

        let req = RequestCode::from_primitive(code);
        match req {
            RequestCode::DnsStart => {
                send_response(send, &mut buf, handler.dns_start().await).await?;
            }
            RequestCode::DnsStop => {
                send_response(send, &mut buf, handler.dns_stop().await).await?;
            }
            RequestCode::Forward => {
                let (target, msg) = codec
                    .deserialize::<(&str, &[u8])>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.forward(target, msg).await;
                match result {
                    Ok(response) => {
                        frame::send_raw(send, &response).await?;
                    }
                    Err(e) => {
                        let err_msg = format!("failed to forward message to {}: {}", target, e);
                        message::send_err(send, &mut buf, err_msg).await?;
                    }
                }
            }
            RequestCode::Reboot => {
                send_response(send, &mut buf, handler.reboot().await).await?;
            }
            RequestCode::ReloadConfig => {
                send_response(send, &mut buf, handler.reload_config().await).await?;
            }
            RequestCode::ReloadTi => {
                let version = codec
                    .deserialize::<&str>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.reload_ti(version).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::ResourceUsage => {
                send_response(send, &mut buf, handler.resource_usage().await).await?;
            }
            RequestCode::TorExitNodeList => {
                let nodes = codec
                    .deserialize::<Vec<&str>>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.tor_exit_node_list(&nodes).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::TrustedDomainList => {
                let domains = codec
                    .deserialize::<Vec<&str>>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.trusted_domain_list(&domains).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::Unknown => {
                let err_msg = format!("unknown request code: {}", code);
                message::send_err(send, &mut buf, err_msg).await?;
            }
        }
    }
    Ok(())
}

async fn send_response<T: Serialize>(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
    body: T,
) -> Result<(), frame::SendError> {
    match frame::send(send, buf, body).await {
        Ok(_) => Ok(()),
        Err(frame::SendError::WriteError(e)) => Err(frame::SendError::WriteError(e)),
        Err(e) => message::send_err(send, buf, e).await,
    }
}
#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use async_trait::async_trait;

    use crate::{
        frame, message,
        test::{channel, TOKEN},
        RequestCode,
    };

    #[tokio::test]
    async fn handle_forward() {
        #[derive(Default)]
        struct TestHandler {
            reboot_count: usize,
        }

        #[async_trait]
        impl super::Handler for TestHandler {
            async fn forward(&mut self, target: &str, msg: &[u8]) -> Result<Vec<u8>, String> {
                let code = u32::from_le_bytes(msg[..size_of::<u32>()].try_into().unwrap());
                let req = RequestCode::from(code);
                let response = format!("forwarded {:?} to {}", req, target);
                Ok(response.as_bytes().to_vec())
            }

            async fn reboot(&mut self) -> Result<(), String> {
                self.reboot_count += 1;
                Ok(())
            }
        }

        let mut handler = TestHandler::default();

        let _lock = TOKEN.lock().await;
        let mut channel = channel().await;

        let mut buf = Vec::new();
        let res =
            message::send_request(&mut channel.client.send, &mut buf, RequestCode::Reboot, ())
                .await;
        assert!(res.is_ok());
        let res = message::send_forward_request(
            &mut channel.client.send,
            &mut buf,
            "agent",
            RequestCode::ReloadTi,
            (),
        )
        .await;
        assert!(res.is_ok());
        channel.client.send.finish().await.unwrap();

        assert_eq!(handler.reboot_count, 0);
        let res = super::handle(
            &mut handler,
            &mut channel.server.send,
            &mut channel.server.recv,
        )
        .await;
        assert!(res.is_ok());
        assert_eq!(handler.reboot_count, 1);

        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, b"forwarded ReloadTi to agent");
    }
}
