//! Automatic partitioning: ESP + swap + root, btrfs (default, with subvolumes) or ext4.

use crate::process::{run, run_status};
use serde::{Deserialize, Serialize};

pub const ESP_SIZE_MIB: u64 = 512;
pub const SWAP_MIN_GIB: u64 = 8;
pub const SWAP_MAX_GIB: u64 = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RootFs {
    Btrfs,
    Ext4,
}

/// Standard Gentoo-style split: `@` kept snapshot-able on its own, `@var`/`@log`
/// carved out so builds/logs don't bloat root snapshots, `@home` for user data.
pub const BTRFS_SUBVOLUMES: &[&str] = &["@", "@home", "@var", "@log"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub disk: String,
    pub esp_size_mib: u64,
    pub swap_gib: u64,
    pub root_fs: RootFs,
}

/// swap = RAM 1:1, clamped to [SWAP_MIN_GIB, SWAP_MAX_GIB].
pub fn swap_size_gib(ram_bytes: u64) -> u64 {
    let ram_gib = ram_bytes / 1024 / 1024 / 1024;
    ram_gib.clamp(SWAP_MIN_GIB, SWAP_MAX_GIB)
}

pub fn plan(disk: &str, root_fs: RootFs, ram_bytes: u64) -> Layout {
    Layout {
        disk: disk.to_string(),
        esp_size_mib: ESP_SIZE_MIB,
        swap_gib: swap_size_gib(ram_bytes),
        root_fs,
    }
}

/// Result of `apply()`: the concrete partition device paths so later steps
/// (stage3 unpack, bootloader install, fstab generation) know where to mount.
#[derive(Debug, Clone)]
pub struct Partitions {
    pub esp: String,
    pub swap: String,
    pub root: String,
}

/// NVMe/mmcblk devices need a `p` before the partition number (`/dev/nvme0n1p1`),
/// plain disks don't (`/dev/sda1`).
fn part_device(disk: &str, index: u32) -> String {
    let needs_p = disk.ends_with(char::is_numeric);
    if needs_p {
        format!("{disk}p{index}")
    } else {
        format!("{disk}{index}")
    }
}

/// Runs the actual `parted`/`mkfs.*`/subvolume-create sequence against `layout.disk`.
/// Destructive — callers must have already confirmed with the user. Wipes any existing
/// partition table on the disk.
pub async fn apply(layout: &Layout) -> crate::Result<Partitions> {
    let disk = &layout.disk;
    let esp_end_mib = 1 + layout.esp_size_mib; // 1MiB alignment gap at the start
    let swap_end_mib = esp_end_mib + layout.swap_gib * 1024;

    run_status("parted", &["--script", disk, "mklabel", "gpt"]).await?;
    run_status(
        "parted",
        &[
            "--script",
            disk,
            "mkpart",
            "ESP",
            "fat32",
            "1MiB",
            &format!("{esp_end_mib}MiB"),
        ],
    )
    .await?;
    run_status("parted", &["--script", disk, "set", "1", "esp", "on"]).await?;
    run_status(
        "parted",
        &[
            "--script",
            disk,
            "mkpart",
            "swap",
            "linux-swap",
            &format!("{esp_end_mib}MiB"),
            &format!("{swap_end_mib}MiB"),
        ],
    )
    .await?;
    run_status(
        "parted",
        &[
            "--script",
            disk,
            "mkpart",
            "root",
            &format!("{swap_end_mib}MiB"),
            "100%",
        ],
    )
    .await?;

    // Re-read the partition table so the new device nodes exist before mkfs.
    run_status("partprobe", &[disk]).await.ok();

    let esp = part_device(disk, 1);
    let swap = part_device(disk, 2);
    let root = part_device(disk, 3);

    run_status("mkfs.vfat", &["-F32", "-n", "ESP", &esp]).await?;
    run_status("mkswap", &["-L", "swap", &swap]).await?;

    match layout.root_fs {
        RootFs::Btrfs => {
            run_status("mkfs.btrfs", &["-L", "root", "-f", &root]).await?;
            create_btrfs_subvolumes(&root).await?;
        }
        RootFs::Ext4 => {
            run_status("mkfs.ext4", &["-L", "root", "-F", &root]).await?;
        }
    }

    Ok(Partitions { esp, swap, root })
}

/// Mounts the fresh btrfs filesystem at a scratch point, creates each entry in
/// `BTRFS_SUBVOLUMES`, then unmounts — the real per-subvolume mounts under the
/// installer's target root happen later via `mount_target`.
async fn create_btrfs_subvolumes(root_partition: &str) -> crate::Result<()> {
    let scratch = "/mnt/gentoo-installer-scratch";
    run_status("mkdir", &["-p", scratch]).await?;
    run_status("mount", &[root_partition, scratch]).await?;

    for subvol in BTRFS_SUBVOLUMES {
        let path = format!("{scratch}/{subvol}");
        run(
            "btrfs",
            &["subvolume", "create", &path],
        )
        .await?;
    }

    run_status("umount", &[scratch]).await?;
    Ok(())
}

/// Mounts `parts` under `target` (`/mnt/gentoo` typically): root subvolume `@` at
/// `target`, `@home`/`@var`/`@log` at their paths, and the ESP at `target/boot`.
/// For ext4, root mounts directly with no subvolume options.
pub async fn mount_target(layout: &Layout, parts: &Partitions, target: &str) -> crate::Result<()> {
    run_status("mkdir", &["-p", target]).await?;

    match layout.root_fs {
        RootFs::Btrfs => {
            run_status(
                "mount",
                &["-o", "subvol=@,compress=zstd:1", &parts.root, target],
            )
            .await?;
            for (subvol, rel) in [("@home", "home"), ("@var", "var"), ("@log", "var/log")] {
                let mountpoint = format!("{target}/{rel}");
                run_status("mkdir", &["-p", &mountpoint]).await?;
                run_status(
                    "mount",
                    &[
                        "-o",
                        &format!("subvol={subvol},compress=zstd:1"),
                        &parts.root,
                        &mountpoint,
                    ],
                )
                .await?;
            }
        }
        RootFs::Ext4 => {
            run_status("mount", &[&parts.root, target]).await?;
        }
    }

    let boot = format!("{target}/boot");
    run_status("mkdir", &["-p", &boot]).await?;
    run_status("mount", &[&parts.esp, &boot]).await?;

    run_status("swapon", &[&parts.swap]).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_clamps_to_min() {
        assert_eq!(swap_size_gib(4 * 1024 * 1024 * 1024), SWAP_MIN_GIB);
    }

    #[test]
    fn swap_clamps_to_max() {
        assert_eq!(swap_size_gib(128 * 1024 * 1024 * 1024), SWAP_MAX_GIB);
    }

    #[test]
    fn swap_matches_ram_in_range() {
        assert_eq!(swap_size_gib(32 * 1024 * 1024 * 1024), 32);
    }

    #[test]
    fn part_device_nvme_gets_p_infix() {
        assert_eq!(part_device("/dev/nvme0n1", 1), "/dev/nvme0n1p1");
    }

    #[test]
    fn part_device_sata_no_infix() {
        assert_eq!(part_device("/dev/sda", 2), "/dev/sda2");
    }
}
