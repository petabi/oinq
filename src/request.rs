//! Request handlers.

use crate::{frame, message, RequestCode};
use async_trait::async_trait;
use bincode::Options;
use ipnet::IpNet;
use num_enum::FromPrimitive;
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    ops::RangeInclusive,
};
use thiserror::Error;

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

#[derive(Debug, Deserialize, Serialize)]
pub struct Process {
    pub user: String,
    pub cpu_usage: f32,
    pub mem_usage: f64,
    pub start_time: i64,
    pub command: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Config {
    Hog(HogConfig),
    Piglet(PigletConfig),
    Reconverge(ReconvergeConfig),
    Crusher(CrusherConfig),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HogConfig {
    pub review_address: SocketAddr,
    pub giganto_address: Option<SocketAddr>,
    pub active_protocols: Option<Vec<String>>,
    pub active_sources: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PigletConfig {
    pub review_address: SocketAddr,
    pub giganto_address: Option<SocketAddr>,
    pub log_options: Option<Vec<String>>,
    pub http_file_types: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReconvergeConfig {
    pub review_address: SocketAddr,
    pub giganto_address: Option<SocketAddr>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CrusherConfig {
    pub review_address: SocketAddr,
    pub giganto_ingest_address: Option<SocketAddr>,
    pub giganto_publish_address: Option<SocketAddr>,
}

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostNetworkGroup {
    pub hosts: Vec<IpAddr>,
    pub networks: Vec<IpNet>,
    pub ip_ranges: Vec<RangeInclusive<IpAddr>>,
}

/// The error type for handling a request.
#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("failed to receive request")]
    RecvError(#[from] frame::RecvError),
    #[error("failed to send response")]
    SendError(#[from] frame::SendError),
}

// IP address, port numbers, and protocols.
pub type TrafficFilterRule = (IpNet, Option<Vec<u16>>, Option<Vec<u16>>);

/// A request handler that can handle a request to an agent.
#[allow(clippy::diverging_sub_expression)]
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

    /// Updates the list of sampling policies.
    async fn sampling_policy_list(&mut self, _policies: &[u8]) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn update_traffic_filter_rules(
        &mut self,
        _rules: &[TrafficFilterRule],
    ) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn get_config(&mut self) -> Result<Config, String> {
        return Err("not supported".to_string());
    }

    async fn set_config(&mut self, _config: Config) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn delete_sampling_policy(&mut self, _policies_id: &[u8]) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn internal_network_list(&mut self, _list: HostNetworkGroup) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn allow_list(&mut self, _list: HostNetworkGroup) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn block_list(&mut self, _list: HostNetworkGroup) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn trusted_user_agent_list(&mut self, _list: &[&str]) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn process_list(&mut self) -> Result<Vec<Process>, String> {
        return Err("not supported".to_string());
    }

    async fn update_semi_supervised_models(&mut self, _list: &[u8]) -> Result<(), String> {
        return Err("not supported".to_string());
    }

    async fn shutdown(&mut self) -> Result<(), String> {
        return Err("not supported".to_string());
    }
}

/// Handles requests to an agent.
///
/// # Errors
///
/// * `HandlerError::RecvError` if the request could not be received
/// * `HandlerError::SendError` if the response could not be sent
#[allow(clippy::too_many_lines)]
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
            RequestCode::SamplingPolicyList => {
                let result = handler.sampling_policy_list(body).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::DeleteSamplingPolicy => {
                let result = handler.delete_sampling_policy(body).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::TrustedDomainList => {
                let domains = codec
                    .deserialize::<Result<Vec<&str>, String>>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;

                let result = if let Ok(domains) = domains {
                    handler.trusted_domain_list(&domains).await
                } else {
                    Err("invalid request".to_string())
                };
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::InternalNetworkList => {
                let network_list = codec
                    .deserialize::<HostNetworkGroup>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.internal_network_list(network_list).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::AllowList => {
                let allow_list = codec
                    .deserialize::<HostNetworkGroup>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.allow_list(allow_list).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::BlockList => {
                let block_list = codec
                    .deserialize::<HostNetworkGroup>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.block_list(block_list).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::EchoRequest => {
                send_response(send, &mut buf, Ok::<(), String>(())).await?;
            }
            RequestCode::TrustedUserAgentList => {
                let user_agent_list = codec
                    .deserialize::<Vec<&str>>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.trusted_user_agent_list(&user_agent_list).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::ReloadFilterRule => {
                let rules = codec
                    .deserialize::<Vec<TrafficFilterRule>>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.update_traffic_filter_rules(&rules).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::GetConfig => {
                send_response(send, &mut buf, handler.get_config().await).await?;
            }
            RequestCode::SetConfig => {
                let conf = codec
                    .deserialize::<Config>(body)
                    .map_err(frame::RecvError::DeserializationFailure)?;
                let result = handler.set_config(conf).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::ProcessList => {
                send_response(send, &mut buf, handler.process_list().await).await?;
            }
            RequestCode::SemiSupervisedModels => {
                let result = handler.update_semi_supervised_models(body).await;
                send_response(send, &mut buf, result).await?;
            }
            RequestCode::Shutdown => {
                send_response(send, &mut buf, handler.shutdown().await).await?;
            }
            RequestCode::Unknown => {
                let err_msg = format!("unknown request code: {code}");
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
        Ok(()) => Ok(()),
        Err(frame::SendError::WriteError(e)) => Err(frame::SendError::WriteError(e)),
        Err(e) => message::send_err(send, buf, e).await,
    }
}
