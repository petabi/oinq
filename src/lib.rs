pub mod frame;
pub mod message;
pub mod request;
#[cfg(test)]
mod test;

use num_enum::{FromPrimitive, IntoPrimitive};
pub use request::{Configuration, Process, ResourceUsage};

/// Numeric representation of the message types.
#[derive(Clone, Copy, Debug, Eq, FromPrimitive, IntoPrimitive, PartialEq)]
#[repr(u32)]
pub enum RequestCode {
    /// Start DNS filtering
    DnsStart = 1,

    /// Stop DNS filtering
    DnsStop = 2,

    /// Forward a request to another agent
    Forward = 3,

    /// Reboot the host
    Reboot = 4,

    /// Reload the configuration
    ReloadConfig = 6,

    /// Fetch the TI database and reload it
    ReloadTi = 5,

    /// Collect resource usage stats
    ResourceUsage = 7,

    /// Update the list of tor exit nodes
    TorExitNodeList = 8,

    /// Update the list of sampling policies
    SamplingPolicyList = 9,

    /// Update traffic filter rules
    ReloadFilterRule = 10,

    /// Get configuration
    GetConfig = 11,

    /// Set Configuration
    SetConfig = 12,

    /// Delete the list of sampling policies
    DeleteSamplingPolicy = 13,

    /// Update the list of Internal network
    InternalNetworkList = 14,

    /// Update the list of allow
    AllowList = 15,

    /// Update the list of block
    BlockList = 16,

    /// Request Echo (for ping)
    EchoRequest = 17,

    /// Update the list of trusted User-agent
    TrustedUserAgentList = 18,

    /// Update the list of trusted domains
    TrustedDomainList = 0,

    /// Collect process list
    ProcessList = 19,

    /// Update the semi-supervised models
    SemiSupervisedModels = 20,

    /// Unknown request
    #[num_enum(default)]
    Unknown = u32::MAX,
}

#[cfg(test)]
mod tests {
    use num_enum::FromPrimitive;

    use super::RequestCode;

    #[test]
    fn serde() {
        assert_eq!(7u32, u32::from(RequestCode::ResourceUsage));
        assert_eq!(RequestCode::ResourceUsage, RequestCode::from_primitive(7));
    }
}
