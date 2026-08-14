//! Automatic partitioning: ESP + swap + root, btrfs (default, with subvolumes) or ext4.
//! Split into `create_partitions`/`format_partitions` (rather than one `apply`) so the
//! `Partition`/`Format` phase split in the spec maps onto real, separately-idempotent
//! steps instead of being a purely cosmetic phase boundary.

use crate::command::CommandRunner;
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
    plan_with_swap(disk, root_fs, swap_size_gib(ram_bytes))
}

/// Manual-partitioning entry point (Advanced setup): same ESP+swap+root shape, but the
/// swap size is whatever the user chose instead of the RAM-based default. Still clamped
/// to [`SWAP_MIN_GIB`, `SWAP_MAX_GIB`] — those floors/ceilings aren't about RAM, they're
/// about what's actually a sane swap size at all.
pub fn plan_with_swap(disk: &str, root_fs: RootFs, swap_gib: u64) -> Layout {
    Layout {
        disk: disk.to_string(),
        esp_size_mib: ESP_SIZE_MIB,
        swap_gib: swap_gib.clamp(SWAP_MIN_GIB, SWAP_MAX_GIB),
        root_fs,
    }
}

/// Result of `create_partitions()`: the concrete partition device paths so later steps
/// (format, stage3 unpack, bootloader install, fstab generation) know where to act.
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

/// Writes a fresh GPT with ESP/swap/root and returns the resulting device paths.
/// Destructive — callers must have already confirmed with the user. Wipes any existing
/// partition table on the disk. Does not format anything; see `format_partitions`.
pub async fn create_partitions(runner: &dyn CommandRunner, layout: &Layout) -> crate::Result<Partitions> {
    let disk = &layout.disk;
    let esp_end_mib = 1 + layout.esp_size_mib; // 1MiB alignment gap at the start
    let swap_end_mib = esp_end_mib + layout.swap_gib * 1024;

    runner.run_status("parted", &["--script", disk, "mklabel", "gpt"]).await?;
    runner
        .run_status("parted", &["--script", disk, "mkpart", "ESP", "fat32", "1MiB", &format!("{esp_end_mib}MiB")])
        .await?;
    runner.run_status("parted", &["--script", disk, "set", "1", "esp", "on"]).await?;
    runner
        .run_status(
            "parted",
            &["--script", disk, "mkpart", "swap", "linux-swap", &format!("{esp_end_mib}MiB"), &format!("{swap_end_mib}MiB")],
        )
        .await?;
    runner
        .run_status("parted", &["--script", disk, "mkpart", "root", &format!("{swap_end_mib}MiB"), "100%"])
        .await?;

    // Re-read the partition table so the new device nodes exist before mkfs.
    runner.run_status("partprobe", &[disk]).await.ok();

    Ok(Partitions { esp: part_device(disk, 1), swap: part_device(disk, 2), root: part_device(disk, 3) })
}

/// Formats each partition per `layout.root_fs`, creating the btrfs subvolume layout if
/// applicable. Assumes `parts` came from `create_partitions` on the same disk.
pub async fn format_partitions(runner: &dyn CommandRunner, layout: &Layout, parts: &Partitions) -> crate::Result<()> {
    runner.run_status("mkfs.vfat", &["-F32", "-n", "ESP", &parts.esp]).await?;
    runner.run_status("mkswap", &["-L", "swap", &parts.swap]).await?;

    match layout.root_fs {
        RootFs::Btrfs => {
            runner.run_status("mkfs.btrfs", &["-L", "root", "-f", &parts.root]).await?;
            create_btrfs_subvolumes(runner, &parts.root).await?;
        }
        RootFs::Ext4 => {
            runner.run_status("mkfs.ext4", &["-L", "root", "-F", &parts.root]).await?;
        }
    }
    Ok(())
}

/// Mounts the fresh btrfs filesystem at a scratch point, creates each entry in
/// `BTRFS_SUBVOLUMES`, then unmounts — the real per-subvolume mounts under the
/// installer's target root happen later via `mount_target`.
async fn create_btrfs_subvolumes(runner: &dyn CommandRunner, root_partition: &str) -> crate::Result<()> {
    let scratch = "/mnt/gentoo-installer-scratch";
    runner.run_status("mkdir", &["-p", scratch]).await?;
    runner.run_status("mount", &[root_partition, scratch]).await?;

    for subvol in BTRFS_SUBVOLUMES {
        let path = format!("{scratch}/{subvol}");
        runner.run("btrfs", &["subvolume", "create", &path]).await?;
    }

    runner.run_status("umount", &[scratch]).await?;
    Ok(())
}

/// Mounts `parts` under `target` (`/mnt/gentoo` typically): root subvolume `@` at
/// `target`, `@home`/`@var`/`@log` at their paths, and the ESP at `target/boot`.
/// For ext4, root mounts directly with no subvolume options.
pub async fn mount_target(runner: &dyn CommandRunner, layout: &Layout, parts: &Partitions, target: &str) -> crate::Result<()> {
    runner.run_status("mkdir", &["-p", target]).await?;

    match layout.root_fs {
        RootFs::Btrfs => {
            runner.run_status("mount", &["-o", "subvol=@,compress=zstd:1", &parts.root, target]).await?;
            for (subvol, rel) in [("@home", "home"), ("@var", "var"), ("@log", "var/log")] {
                let mountpoint = format!("{target}/{rel}");
                runner.run_status("mkdir", &["-p", &mountpoint]).await?;
                runner
                    .run_status("mount", &["-o", &format!("subvol={subvol},compress=zstd:1"), &parts.root, &mountpoint])
                    .await?;
            }
        }
        RootFs::Ext4 => {
            runner.run_status("mount", &[&parts.root, target]).await?;
        }
    }

    let boot = format!("{target}/boot");
    runner.run_status("mkdir", &["-p", &boot]).await?;
    runner.run_status("mount", &[&parts.esp, &boot]).await?;

    runner.run_status("swapon", &[&parts.swap]).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

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

    #[tokio::test]
    async fn create_partitions_issues_expected_argv() {
        let runner = FakeCommandRunner::new();
        let layout = plan("/dev/sda", RootFs::Btrfs, 16 * 1024 * 1024 * 1024);
        let parts = create_partitions(&runner, &layout).await.unwrap();

        assert_eq!(parts.esp, "/dev/sda1");
        assert_eq!(parts.swap, "/dev/sda2");
        assert_eq!(parts.root, "/dev/sda3");

        runner.assert_call(0, "parted", &["--script", "/dev/sda", "mklabel", "gpt"]);
        runner.assert_call(1, "parted", &["--script", "/dev/sda", "mkpart", "ESP", "fat32", "1MiB", "513MiB"]);
        runner.assert_call(2, "parted", &["--script", "/dev/sda", "set", "1", "esp", "on"]);
    }

    #[tokio::test]
    async fn format_partitions_btrfs_creates_subvolumes() {
        let runner = FakeCommandRunner::new();
        let layout = plan("/dev/sda", RootFs::Btrfs, 16 * 1024 * 1024 * 1024);
        let parts = Partitions { esp: "/dev/sda1".into(), swap: "/dev/sda2".into(), root: "/dev/sda3".into() };

        format_partitions(&runner, &layout, &parts).await.unwrap();

        let calls = runner.calls();
        assert!(calls.iter().any(|(cmd, args)| cmd == "mkfs.btrfs" && args.contains(&"/dev/sda3".to_string())));
        let subvol_calls: Vec<_> = calls
            .iter()
            .filter(|(cmd, args)| cmd == "btrfs" && args.first().map(String::as_str) == Some("subvolume"))
            .collect();
        assert_eq!(subvol_calls.len(), BTRFS_SUBVOLUMES.len());
    }
}
