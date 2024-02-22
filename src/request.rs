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

    async fn update_traffic_filter_rules(&mut self, _rules: &[IpNet]) -> Result<(), String> {
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
                        let err_msg = format!("failed to forward message to {target}: {e}");
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
                    .deserialize::<Vec<IpNet>>(body)
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
#[cfg(test)]
mod tests {
    use crate::{
        frame, message,
        request::HostNetworkGroup,
        test::{channel, TOKEN},
        Process, RequestCode,
    };
    use async_trait::async_trait;
    use ipnet::IpNet;
    use std::{
        mem::size_of,
        net::{IpAddr, Ipv4Addr},
        ops::RangeInclusive,
        str::FromStr,
    };

    #[tokio::test]
    async fn handle_forward() {
        #[derive(Default)]
        struct TestHandler {
            reboot_count: usize,
            filter_rules: usize,
            trusted_domains: usize,
            internal_network_list: usize,
            allow_list: usize,
            block_list: usize,
            trusted_user_agents: usize,
            process_list_cnt: usize,
            shutdown_count: usize,
            set_config_cnt: usize,
        }

        #[async_trait]
        impl super::Handler for TestHandler {
            async fn forward(&mut self, target: &str, msg: &[u8]) -> Result<Vec<u8>, String> {
                let code = u32::from_le_bytes(msg[..size_of::<u32>()].try_into().unwrap());
                let req = RequestCode::from(code);
                let response = format!("forwarded {req:?} to {target}");
                Ok(response.as_bytes().to_vec())
            }

            async fn reboot(&mut self) -> Result<(), String> {
                self.reboot_count += 1;
                Ok(())
            }

            async fn update_traffic_filter_rules(&mut self, rules: &[IpNet]) -> Result<(), String> {
                self.filter_rules = rules.len();
                Ok(())
            }

            async fn trusted_domain_list(&mut self, domains: &[&str]) -> Result<(), String> {
                self.trusted_domains = domains.len();
                Ok(())
            }

            async fn internal_network_list(
                &mut self,
                network_list: HostNetworkGroup,
            ) -> Result<(), String> {
                self.internal_network_list = network_list.hosts.len();
                Ok(())
            }

            async fn allow_list(&mut self, allow_list: HostNetworkGroup) -> Result<(), String> {
                self.allow_list = allow_list.networks.len();
                Ok(())
            }

            async fn block_list(&mut self, block_list: HostNetworkGroup) -> Result<(), String> {
                self.block_list = block_list.ip_ranges.len();
                Ok(())
            }

            async fn trusted_user_agent_list(&mut self, list: &[&str]) -> Result<(), String> {
                self.trusted_user_agents = list.len();
                Ok(())
            }

            async fn process_list(&mut self) -> Result<Vec<Process>, String> {
                self.process_list_cnt += 1;
                Ok(Vec::new())
            }

            async fn shutdown(&mut self) -> Result<(), String> {
                self.shutdown_count += 1;
                Ok(())
            }

            async fn set_config(&mut self, _config: super::Config) -> Result<(), String> {
                self.set_config_cnt += 1;
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

        let rules = vec![
            IpNet::from_str("192.168.1.0/24").unwrap(),
            IpNet::from_str("10.80.10.10/32").unwrap(),
        ];

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::ReloadFilterRule,
            rules,
        )
        .await;
        assert!(res.is_ok());

        let trusted_domains: Result<Vec<String>, String> =
            Ok(vec![".google.com".to_string(), ".gstatic.com".to_string()]);

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::TrustedDomainList,
            trusted_domains,
        )
        .await;
        assert!(res.is_ok());

        let input_internal_list = HostNetworkGroup {
            hosts: vec![
                IpAddr::V4(Ipv4Addr::new(10, 0, 9, 1)),
                IpAddr::V4(Ipv4Addr::new(10, 0, 9, 2)),
                IpAddr::V4(Ipv4Addr::new(10, 0, 9, 3)),
            ],
            networks: Vec::new(),
            ip_ranges: Vec::new(),
        };

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::InternalNetworkList,
            input_internal_list,
        )
        .await;
        assert!(res.is_ok());

        let input_allow_list = HostNetworkGroup {
            hosts: Vec::new(),
            networks: vec![
                IpNet::from_str("192.168.1.0/24").unwrap(),
                IpNet::from_str("10.80.10.10/32").unwrap(),
            ],
            ip_ranges: Vec::new(),
        };

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::AllowList,
            input_allow_list,
        )
        .await;
        assert!(res.is_ok());

        let input_block_list = HostNetworkGroup {
            hosts: Vec::new(),
            networks: Vec::new(),
            ip_ranges: vec![RangeInclusive::new(
                IpAddr::V4(Ipv4Addr::new(10, 80, 10, 10)),
                IpAddr::V4(Ipv4Addr::new(10, 80, 10, 20)),
            )],
        };

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::BlockList,
            input_block_list,
        )
        .await;
        assert!(res.is_ok());

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::EchoRequest,
            (),
        )
        .await;
        assert!(res.is_ok());

        let trusted_user_agent_list: Vec<String> = vec![
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/114.0"
                .to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/113."
                .to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/113.0.0.0 Safari/537.36 Edg/113.0.1774.35"
                .to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/113.0.0.0 Safari/537.36 Edg/113.0.1774.42"
                .to_string(),
        ];

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::TrustedUserAgentList,
            trusted_user_agent_list,
        )
        .await;
        assert!(res.is_ok());

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::ProcessList,
            (),
        )
        .await;
        assert!(res.is_ok());

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::Shutdown,
            (),
        )
        .await;
        assert!(res.is_ok());
        let hog_config = super::Config::Hog(super::HogConfig {
            review_address: "127.0.0.1:1234".parse().unwrap(),
            giganto_address: None,
            active_protocols: None,
            active_sources: None,
        });

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::SetConfig,
            hog_config,
        )
        .await;
        assert!(res.is_ok());

        let reconverge_config = super::Config::Reconverge(super::ReconvergeConfig {
            review_address: "127.0.0.1:1234".parse().unwrap(),
            giganto_address: "127.0.0.1:2345".parse().ok(),
        });

        let res = message::send_request(
            &mut channel.client.send,
            &mut buf,
            RequestCode::SetConfig,
            reconverge_config,
        )
        .await;
        assert!(res.is_ok());

        channel.client.send.finish().await.unwrap();

        assert_eq!(handler.reboot_count, 0);
        assert_eq!(handler.filter_rules, 0);
        assert_eq!(handler.trusted_domains, 0);
        assert_eq!(handler.internal_network_list, 0);
        assert_eq!(handler.allow_list, 0);
        assert_eq!(handler.block_list, 0);
        assert_eq!(handler.trusted_user_agents, 0);
        assert_eq!(handler.process_list_cnt, 0);
        assert_eq!(handler.shutdown_count, 0);
        assert_eq!(handler.set_config_cnt, 0);
        let res = super::handle(
            &mut handler,
            &mut channel.server.send,
            &mut channel.server.recv,
        )
        .await;
        assert!(res.is_ok());
        assert_eq!(handler.reboot_count, 1);
        assert_eq!(handler.filter_rules, 2);
        assert_eq!(handler.trusted_domains, 2);
        assert_eq!(handler.internal_network_list, 3);
        assert_eq!(handler.allow_list, 2);
        assert_eq!(handler.block_list, 1);
        assert_eq!(handler.trusted_user_agents, 4);
        assert_eq!(handler.process_list_cnt, 1);
        assert_eq!(handler.shutdown_count, 1);
        assert_eq!(handler.set_config_cnt, 2);

        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        frame::recv_raw(&mut channel.client.recv, &mut buf)
            .await
            .unwrap();
        assert_eq!(buf, b"forwarded ReloadTi to agent");
    }
}
