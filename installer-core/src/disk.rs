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
    let raw = runner
        .run("lsblk", &["-J", "-b", "-o", "NAME,SIZE,MODEL,TYPE"])
        .await?;
    let parsed: LsblkOutput =
        serde_json::from_str(&raw).map_err(|e| crate::Error::Other(e.into()))?;

    Ok(parsed
        .blockdevices
        .into_iter()
        // zram/loop show up as type "disk" too but aren't real install targets.
        .filter(|d| {
            d.device_type == "disk" && !d.name.starts_with("zram") && !d.name.starts_with("loop")
        })
        .map(|d| Disk {
            path: format!("/dev/{}", d.name),
            size_bytes: d.size.unwrap_or(0),
            model: d.model.unwrap_or_default().trim().to_string(),
        })
        .collect())
}

/// One partition of a disk, as `parted` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct PartInfo {
    pub number: u32,
    pub device: String,
    pub start_mib: f64,
    pub end_mib: f64,
    /// `ext4`, `ntfs`, `fat32`, `btrfs`...; empty when `parted` does not know it
    pub fs: String,
    pub name: String,
    pub is_esp: bool,
}

impl PartInfo {
    pub fn size_mib(&self) -> f64 {
        self.end_mib - self.start_mib
    }
}

/// A stretch of the disk no partition uses (whole MiB, rounded inwards).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeRegion {
    pub start_mib: u64,
    pub end_mib: u64,
}

impl FreeRegion {
    pub fn size_mib(&self) -> u64 {
        self.end_mib - self.start_mib
    }
}

/// What a disk holds now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inspection {
    /// `gpt`, `msdos` or empty (no partition table)
    pub table: String,
    pub partitions: Vec<PartInfo>,
    pub free: Vec<FreeRegion>,
}

impl Inspection {
    pub fn esp(&self) -> Option<&PartInfo> {
        self.partitions.iter().find(|p| p.is_esp)
    }

    /// The largest free region.
    pub fn largest_free(&self) -> Option<FreeRegion> {
        self.free.iter().copied().max_by_key(|r| r.size_mib())
    }
}

fn mib(field: &str) -> f64 {
    field.trim_end_matches("MiB").parse().unwrap_or(0.0)
}

/// Parses `parted -m -s <disk> unit MiB print free`.
pub fn parse_parted(disk: &str, text: &str) -> Inspection {
    let mut out = Inspection::default();
    for line in text.lines() {
        let line = line.trim_end_matches(';');
        let f: Vec<&str> = line.split(':').collect();
        if f.first() == Some(&disk) {
            out.table = f.get(5).copied().unwrap_or("").to_string();
            continue;
        }
        if f.len() < 5 || f[0].parse::<u32>().is_err() {
            continue;
        }
        let (start, end) = (mib(f[1]), mib(f[2]));
        if f[4] == "free" {
            let (start, end) = (start.ceil() as u64, end.floor() as u64);
            if end > start {
                out.free.push(FreeRegion {
                    start_mib: start,
                    end_mib: end,
                });
            }
            continue;
        }
        let number: u32 = f[0].parse().unwrap_or(0);
        out.partitions.push(PartInfo {
            number,
            device: crate::partition::part_device(disk, number),
            start_mib: start,
            end_mib: end,
            fs: f[4].to_string(),
            name: f.get(5).copied().unwrap_or("").to_string(),
            is_esp: f
                .get(6)
                .is_some_and(|flags| flags.split(',').any(|x| x.trim() == "esp")),
        });
    }
    out
}

/// Reads the partition table of `disk` (read-only).
pub async fn inspect(runner: &dyn CommandRunner, disk: &str) -> crate::Result<Inspection> {
    let text = runner
        .run(
            "parted",
            &["-m", "-s", disk, "unit", "MiB", "print", "free"],
        )
        .await?;
    Ok(parse_parted(disk, &text))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parted_output_gives_partitions_the_esp_and_the_free_regions() {
        let text = "BYT;\n/dev/sda:476940MiB:scsi:512:512:gpt:ATA Disk:;\n1:0.02MiB:1.00MiB:0.98MiB:free;\n1:1.00MiB:261MiB:260MiB:fat32:EFI system partition:boot, esp;\n2:261MiB:133261MiB:133000MiB:ntfs:Basic data partition:msftdata;\n1:133261MiB:476940MiB:343679MiB:free;\n";
        let i = parse_parted("/dev/sda", text);
        assert_eq!(i.table, "gpt");
        assert_eq!(i.partitions.len(), 2);
        assert_eq!(i.esp().unwrap().device, "/dev/sda1");
        assert_eq!(i.partitions[1].fs, "ntfs");
        assert_eq!(
            i.largest_free(),
            Some(FreeRegion {
                start_mib: 133261,
                end_mib: 476940
            })
        );
        assert!(
            !i.free.contains(&FreeRegion {
                start_mib: 1,
                end_mib: 1
            }),
            "a gap under a MiB is not a region"
        );
    }

    #[test]
    fn a_blank_disk_has_no_table_and_one_free_region() {
        let text = "BYT;\n/dev/vdb:10240MiB:virtio:512:512:unknown:Virtio Block Device:;\n1:0.00MiB:10240MiB:10240MiB:free;\n";
        let i = parse_parted("/dev/vdb", text);
        assert_eq!(i.table, "unknown");
        assert!(i.partitions.is_empty());
        assert_eq!(
            i.free,
            vec![FreeRegion {
                start_mib: 0,
                end_mib: 10240
            }]
        );
    }
}
