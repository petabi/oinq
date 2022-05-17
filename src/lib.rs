pub mod frame;

use num_enum::TryFromPrimitive;
use serde::{Deserialize, Serialize};

/// Numeric representation of the message types.
#[derive(Clone, Copy, Debug, Deserialize, TryFromPrimitive, Serialize)]
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

    /// Reload the configuration
    ReloadConfig = 6,

    /// Fetch the TI database and reload it
    ReloadTi = 5,

    /// Collect resource usage stats
    ResourceUsage = 7,

    /// Update the list of trusted domains
    TrustedDomainList = 0,
}
