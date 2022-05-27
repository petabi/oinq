//! Request handlers.

use std::path::Path;

use async_trait::async_trait;
use bincode::Options;
use gethostname::gethostname;
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{frame, message, RequestCode};

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

    async fn reboot(&mut self) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn reload_config(&mut self) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn reload_ti(&mut self, _version: &str) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn resource_usage(&mut self) -> Result<(), String> {
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

        match code {
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
            RequestCode::TrustedDomainList => {
                let domains = codec
                    .deserialize::<Vec<&str>>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.trusted_domain_list(&domains).await;
                send_response(send, &mut buf, result).await?;
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

/// Sends resource usage stats to the agent.
///
/// # Errors
///
/// * `frame::SendError` if the message could not be sent
pub async fn handle_resource_usage(
    send: &mut SendStream,
    buf: &mut Vec<u8>,
) -> Result<(), frame::SendError> {
    let usage = resource_usage().await;
    frame::send(
        send,
        buf,
        Ok((gethostname().to_string_lossy().into_owned(), usage))
            as Result<(String, ResourceUsage), &str>,
    )
    .await
}

async fn resource_usage() -> ResourceUsage {
    use sysinfo::{DiskExt, ProcessorExt, RefreshKind, System, SystemExt};

    let refresh = RefreshKind::new()
        .with_cpu()
        .with_disks_list()
        .with_memory();
    let mut system = System::new_with_specifics(refresh);

    let (total_disk_space, used_disk_space) = {
        let disks = system.disks();
        if let Some(d) = disks
            .iter()
            .find(|&disk| disk.mount_point() == Path::new("/data"))
        {
            (d.total_space(), d.total_space() - d.available_space())
        } else {
            // Find the disk with the largest space if `/data` is not found
            if let Some(d) = disks.iter().max_by_key(|&disk| disk.total_space()) {
                (d.total_space(), d.total_space() - d.available_space())
            } else {
                (0, 0)
            }
        }
    };

    // Calculating CPU usage requires a time interval.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    system.refresh_cpu();

    ResourceUsage {
        cpu_usage: system.global_processor_info().cpu_usage(),
        total_memory: system.total_memory(),
        used_memory: system.used_memory(),
        total_disk_space,
        used_disk_space,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        frame, message,
        test::{channel, TOKEN},
        RequestCode,
    };

    #[tokio::test]
    async fn handle() {
        use async_trait::async_trait;

        #[derive(Default)]
        struct TestHandler {
            reboot_count: usize,
        }

        #[async_trait]
        impl super::Handler for TestHandler {
            async fn forward(&mut self, target: &str, msg: &[u8]) -> Result<Vec<u8>, String> {
                let code = bincode::deserialize::<RequestCode>(msg).unwrap();
                let response = format!("forwarded {:?} to {}", code, target);
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
        let res = frame::send_raw(&mut channel.client.send, b"\xff\xff\xff\xff").await;
        assert!(res.is_ok());

        assert_eq!(handler.reboot_count, 0);
        let res = super::handle(
            &mut handler,
            &mut channel.server.send,
            &mut channel.server.recv,
        )
        .await;
        assert_eq!(handler.reboot_count, 1);

        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, b"forwarded ReloadTi to agent");
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn resource_usage() {
        let usage = super::resource_usage().await;
        assert!(0.0 <= usage.cpu_usage && usage.cpu_usage <= 100.0);
        assert!(usage.used_memory <= usage.total_memory);
        assert!(usage.used_disk_space <= usage.total_disk_space);
    }
}
