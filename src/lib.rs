pub mod frame;
pub mod message;
pub mod request;
#[cfg(test)]
mod test;

use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};

/// Numeric representation of the message types.
#[derive(
    Clone, Copy, Debug, Deserialize, IntoPrimitive, PartialEq, Serialize, TryFromPrimitive,
)]
#[serde(into = "u32", try_from = "u32")]
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

    /// Update the list of trusted domains
    TrustedDomainList = 0,
}

#[cfg(test)]
mod tests {
    use super::RequestCode;

    #[test]
    fn serde() {
        let serialized = bincode::serialize(&RequestCode::ResourceUsage).unwrap();
        assert_eq!(
            u32::from_le_bytes(serialized.clone().try_into().expect("4 bytes")),
            RequestCode::ResourceUsage.into()
        );
        let code = bincode::deserialize::<u32>(&serialized).unwrap();
        assert_eq!(code, RequestCode::ResourceUsage.into());
    }
}
