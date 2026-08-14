//! Lists candidate installation disks (whole block devices, not partitions/loop/rom).

use crate::command::CommandRunner;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Disk {
    pub path: String,
    pub size_bytes: u64,
    pub model: String,
}

#[derive(Deserialize)]
struct LsblkOutput {
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    // `lsblk -b` (bytes, no human-readable suffix) emits SIZE as a JSON number, not a
    // string — only the human-readable form (no -b) quotes it.
    size: Option<u64>,
    model: Option<String>,
    #[serde(rename = "type")]
    device_type: String,
}

pub async fn list(runner: &dyn CommandRunner) -> crate::Result<Vec<Disk>> {
    let raw = runner.run("lsblk", &["-J", "-b", "-o", "NAME,SIZE,MODEL,TYPE"]).await?;
    let parsed: LsblkOutput =
        serde_json::from_str(&raw).map_err(|e| crate::Error::Other(e.into()))?;

    Ok(parsed
        .blockdevices
        .into_iter()
        // zram/loop show up as type "disk" too but aren't real install targets.
        .filter(|d| d.device_type == "disk" && !d.name.starts_with("zram") && !d.name.starts_with("loop"))
        .map(|d| Disk {
            path: format!("/dev/{}", d.name),
            size_bytes: d.size.unwrap_or(0),
            model: d.model.unwrap_or_default().trim().to_string(),
        })
        .collect())
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}
