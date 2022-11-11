pub mod frame;
pub mod message;
pub mod request;
#[cfg(test)]
mod test;

use num_enum::{FromPrimitive, IntoPrimitive};
pub use request::ResourceUsage;

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

    /// Update the list of trusted domains
    TrustedDomainList = 0,

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
