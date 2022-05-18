//! Request handlers.

use std::path::Path;

use gethostname::gethostname;
use quinn::SendStream;
use serde::{Deserialize, Serialize};

use crate::frame;

#[derive(Debug, Deserialize, Serialize)]
pub struct ResourceUsage {
    pub cpu_usage: f32,
    pub total_memory: u64,
    pub used_memory: u64,
    pub total_disk_space: u64,
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
    #[tokio::test]
    async fn resource_usage() {
        let usage = super::resource_usage().await;
        assert!(0.0 <= usage.cpu_usage && usage.cpu_usage <= 100.0);
        assert!(usage.used_memory <= usage.total_memory);
        assert!(usage.used_disk_space <= usage.total_disk_space);
    }
}
