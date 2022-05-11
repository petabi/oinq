pub mod frame;

use num_enum::TryFromPrimitive;
use serde::Deserialize;

/// Numeric representation of the message types.
#[derive(Clone, Copy, Debug, Deserialize, TryFromPrimitive)]
#[serde(try_from = "u32")]
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

    /// Fetch the TI database and reload it
    ReloadTi = 5,

    /// Update the list of trusted domains
    TrustedDomainList = 0,
}
