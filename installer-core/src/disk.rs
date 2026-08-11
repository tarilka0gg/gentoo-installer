//! Lists candidate installation disks (whole block devices, not partitions/loop/rom).

use crate::process::run;
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
    size: Option<String>,
    model: Option<String>,
    #[serde(rename = "type")]
    device_type: String,
}

pub async fn list() -> crate::Result<Vec<Disk>> {
    let raw = run("lsblk", &["-J", "-b", "-o", "NAME,SIZE,MODEL,TYPE"]).await?;
    let parsed: LsblkOutput =
        serde_json::from_str(&raw).map_err(|e| crate::Error::Other(e.into()))?;

    Ok(parsed
        .blockdevices
        .into_iter()
        .filter(|d| d.device_type == "disk")
        .map(|d| Disk {
            path: format!("/dev/{}", d.name),
            size_bytes: d.size.and_then(|s| s.parse().ok()).unwrap_or(0),
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
