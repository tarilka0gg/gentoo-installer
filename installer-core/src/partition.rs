//! Automatic partitioning: ESP + swap + root + home, btrfs (default, with subvolumes) or ext4. `/` and `/home` are separate
//! partitions in a fixed proportion (see [`root_home_split`]) so that a full home cannot starve the system of space (and the
//! reverse), the usual way a btrfs system runs into ENOSPC and corrupts data.
//! Split into `create_partitions`/`format_partitions` (rather than one `apply`) so the
//! `Partition`/`Format` phase split in the spec maps onto real, separately-idempotent
//! steps instead of being a purely cosmetic phase boundary.

use crate::command::CommandRunner;
use serde::{Deserialize, Serialize};

pub const ESP_SIZE_MIB: u64 = 512;
pub const SWAP_MIN_GIB: u64 = 8;
pub const SWAP_MAX_GIB: u64 = 96;

/// Below this disk size the system gets a bigger share (40 %) of what is left after ESP and swap: 20 % of a small disk is too
/// little for a desktop.
pub const SMALL_DISK_GIB: u64 = 128;
/// The system partition never gets less than this.
pub const ROOT_MIN_GIB: u64 = 20;
/// What is left after ESP and swap must be at least this for `/home` to be a partition of its own; below it everything stays in root.
pub const SPLIT_MIN_GIB: u64 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RootFs {
    Btrfs,
    Ext4,
}

/// Standard Gentoo-style split: `@` kept snapshot-able on its own, `@var`/`@log`
/// carved out so builds/logs don't bloat root snapshots, `@home` for user data.
pub const BTRFS_SUBVOLUMES: &[&str] = &["@", "@home", "@var", "@log"];
/// The same without `@home`, when `/home` is a partition of its own (its filesystem has the `@home` subvolume).
pub const BTRFS_ROOT_SUBVOLUMES: &[&str] = &["@", "@var", "@log"];

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub disk: String,
    pub esp_size_mib: u64,
    pub swap_gib: u64,
    pub root_fs: RootFs,
    /// `/home` on a partition of its own when the disk is big enough (see [`root_home_split`]).
    #[serde(default = "default_true")]
    pub separate_home: bool,
}

/// `(root_mib, home_mib)` for a disk of `disk_bytes` with the ESP and swap in front, or `None` when everything should stay in one
/// root partition (an unknown or small disk). The system gets 20 % of what is left after ESP and swap, 40 % when the disk is smaller than
/// 128 GiB, and never less than [`ROOT_MIN_GIB`]; `/home` gets the rest.
pub fn root_home_split(disk_bytes: u64, esp_mib: u64, swap_gib: u64) -> Option<(u64, u64)> {
    let disk_mib = disk_bytes / (1024 * 1024);
    let used = 1 + esp_mib + swap_gib * 1024 + 1; // alignment gap in front, GPT backup behind
    let rest = disk_mib.checked_sub(used)?;
    if rest < SPLIT_MIN_GIB * 1024 {
        return None;
    }
    let share = if disk_mib < SMALL_DISK_GIB * 1024 {
        40
    } else {
        20
    };
    let root = (rest * share / 100).max(ROOT_MIN_GIB * 1024);
    let home = rest.checked_sub(root)?;
    (home >= 10 * 1024).then_some((root, home))
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
        separate_home: true,
    }
}

/// Result of `create_partitions()`: the concrete partition device paths so later steps
/// (format, stage3 unpack, bootloader install, fstab generation) know where to act.
#[derive(Debug, Clone)]
pub struct Partitions {
    pub esp: String,
    pub swap: String,
    pub root: String,
    /// `/home`'s partition, when it has one.
    pub home: Option<String>,
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

/// The device paths [`create_partitions`] produces for `layout`, without touching the disk: the names
/// follow only from the disk and the fixed ESP/swap/root order. A resumed install needs them again and
/// cannot get them from the partitioning step it is skipping.
pub fn partitions_for(layout: &Layout) -> Partitions {
    // A fourth partition exists only when the split applied to this disk: a resumed install finds out by the device node.
    let home = part_device(&layout.disk, 4);
    let has_home = layout.separate_home && std::path::Path::new(&home).exists();
    Partitions {
        esp: part_device(&layout.disk, 1),
        swap: part_device(&layout.disk, 2),
        root: part_device(&layout.disk, 3),
        home: has_home.then_some(home),
    }
}

/// Size of the disk in bytes (`0` when it cannot be read).
async fn disk_bytes(runner: &dyn CommandRunner, disk: &str) -> u64 {
    runner
        .run("blockdev", &["--getsize64", disk])
        .await
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Writes a fresh GPT with ESP/swap/root and returns the resulting device paths.
/// Destructive — callers must have already confirmed with the user. Wipes any existing
/// partition table on the disk. Does not format anything; see `format_partitions`.
pub async fn create_partitions(
    runner: &dyn CommandRunner,
    layout: &Layout,
) -> crate::Result<Partitions> {
    let disk = &layout.disk;
    let esp_end_mib = 1 + layout.esp_size_mib; // 1MiB alignment gap at the start
    let swap_end_mib = esp_end_mib + layout.swap_gib * 1024;

    runner
        .run_status("parted", &["--script", disk, "mklabel", "gpt"])
        .await?;
    runner
        .run_status(
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
    runner
        .run_status("parted", &["--script", disk, "set", "1", "esp", "on"])
        .await?;
    runner
        .run_status(
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
    // `/` and `/home` side by side when the disk is big enough, one root partition otherwise.
    let split = if layout.separate_home {
        root_home_split(
            disk_bytes(runner, disk).await,
            layout.esp_size_mib,
            layout.swap_gib,
        )
    } else {
        None
    };
    let root_end = match split {
        Some((root_mib, _)) => format!("{}MiB", swap_end_mib + root_mib),
        None => "100%".to_string(),
    };
    runner
        .run_status(
            "parted",
            &[
                "--script",
                disk,
                "mkpart",
                "root",
                &format!("{swap_end_mib}MiB"),
                &root_end,
            ],
        )
        .await?;
    if split.is_some() {
        runner
            .run_status(
                "parted",
                &["--script", disk, "mkpart", "home", &root_end, "100%"],
            )
            .await?;
    }

    // Re-read the partition table so the new device nodes exist before mkfs.
    runner.run_status("partprobe", &[disk]).await.ok();

    Ok(Partitions {
        home: split.map(|_| part_device(disk, 4)),
        ..partitions_for(layout)
    })
}

/// Formats each partition per `layout.root_fs`, creating the btrfs subvolume layout if
/// applicable. Assumes `parts` came from `create_partitions` on the same disk.
pub async fn format_partitions(
    runner: &dyn CommandRunner,
    layout: &Layout,
    parts: &Partitions,
) -> crate::Result<()> {
    runner
        .run_status("mkfs.vfat", &["-F32", "-n", "ESP", &parts.esp])
        .await?;
    runner
        .run_status("mkswap", &["-L", "swap", &parts.swap])
        .await?;

    match layout.root_fs {
        RootFs::Btrfs => {
            runner
                .run_status("mkfs.btrfs", &["-L", "root", "-f", &parts.root])
                .await?;
            let subvolumes = if parts.home.is_some() {
                BTRFS_ROOT_SUBVOLUMES
            } else {
                BTRFS_SUBVOLUMES
            };
            create_btrfs_subvolumes(runner, &parts.root, subvolumes).await?;
            if let Some(home) = &parts.home {
                runner
                    .run_status("mkfs.btrfs", &["-L", "home", "-f", home])
                    .await?;
                create_btrfs_subvolumes(runner, home, &["@home"]).await?;
            }
        }
        RootFs::Ext4 => {
            runner
                .run_status("mkfs.ext4", &["-L", "root", "-F", &parts.root])
                .await?;
            if let Some(home) = &parts.home {
                runner
                    .run_status("mkfs.ext4", &["-L", "home", "-F", home])
                    .await?;
            }
        }
    }

    // `mkfs.vfat` in particular doesn't guarantee its writes reach the block device
    // before returning — live-tested against a loopback device, mounting the ESP right
    // after formatting failed with "wrong fs type, bad superblock" despite `blkid`
    // showing a perfectly valid vfat filesystem moments later. A `sync` here (cheap,
    // always safe) closes that race instead of leaving `mount_target` to hit it
    // nondeterministically depending on how much other work happens in between.
    runner.run_status("sync", &[]).await.ok();

    Ok(())
}

/// Mounts the fresh btrfs filesystem at a scratch point, creates each entry in
/// `BTRFS_SUBVOLUMES`, then unmounts — the real per-subvolume mounts under the
/// installer's target root happen later via `mount_target`.
async fn create_btrfs_subvolumes(
    runner: &dyn CommandRunner,
    root_partition: &str,
    subvolumes: &[&str],
) -> crate::Result<()> {
    let scratch = "/mnt/gentoo-installer-scratch";
    runner.run_status("mkdir", &["-p", scratch]).await?;
    runner
        .run_status("mount", &[root_partition, scratch])
        .await?;

    for subvol in subvolumes {
        let path = format!("{scratch}/{subvol}");
        runner.run("btrfs", &["subvolume", "create", &path]).await?;
    }

    runner.run_status("umount", &[scratch]).await?;
    Ok(())
}

/// Mounts `parts` under `target` (`/mnt/gentoo` typically): root subvolume `@` at
/// `target`, `@home`/`@var`/`@log` at their paths, and the ESP at `target/boot`.
/// For ext4, root mounts directly with no subvolume options.
pub async fn mount_target(
    runner: &dyn CommandRunner,
    layout: &Layout,
    parts: &Partitions,
    target: &str,
) -> crate::Result<()> {
    runner.run_status("mkdir", &["-p", target]).await?;

    match layout.root_fs {
        RootFs::Btrfs => {
            runner
                .run_status(
                    "mount",
                    &["-o", "subvol=@,compress=zstd:1", &parts.root, target],
                )
                .await?;
            // (source device, subvolume, mount point): `/home` comes from its own partition when there is one.
            let mut mounts = vec![
                (&parts.root, "@var", "var"),
                (&parts.root, "@log", "var/log"),
            ];
            mounts.push((parts.home.as_ref().unwrap_or(&parts.root), "@home", "home"));
            for (device, subvol, rel) in mounts {
                let mountpoint = format!("{target}/{rel}");
                runner.run_status("mkdir", &["-p", &mountpoint]).await?;
                runner
                    .run_status(
                        "mount",
                        &[
                            "-o",
                            &format!("subvol={subvol},compress=zstd:1"),
                            device,
                            &mountpoint,
                        ],
                    )
                    .await?;
            }
        }
        RootFs::Ext4 => {
            runner.run_status("mount", &[&parts.root, target]).await?;
            if let Some(home) = &parts.home {
                let mountpoint = format!("{target}/home");
                runner.run_status("mkdir", &["-p", &mountpoint]).await?;
                runner.run_status("mount", &[home, &mountpoint]).await?;
            }
        }
    }

    let boot = format!("{target}/boot");
    runner.run_status("mkdir", &["-p", &boot]).await?;
    // `iocharset=utf8` avoids a real, live-tested failure mode: the kernel's FAT driver
    // defaults to iso8859-1 for filename charset conversion, which needs the
    // `nls_iso8859-1` module built — plenty of minimal/custom kernel configs don't build
    // it (confirmed on this dev machine's own kernel), and the resulting mount failure
    // ("wrong fs type, bad option, bad superblock") gives no hint that charset support is
    // the actual cause; `dmesg` says "IO charset iso8859-1 not found", `mount(8)` doesn't.
    // utf8 needs no NLS module at all.
    runner
        .run_status("mount", &["-o", "iocharset=utf8", &parts.esp, &boot])
        .await?;

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
        runner.assert_call(
            1,
            "parted",
            &[
                "--script", "/dev/sda", "mkpart", "ESP", "fat32", "1MiB", "513MiB",
            ],
        );
        runner.assert_call(
            2,
            "parted",
            &["--script", "/dev/sda", "set", "1", "esp", "on"],
        );
    }

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn a_big_disk_gives_the_system_a_fifth_and_home_the_rest() {
        // 1 TB: ESP 512 MiB, swap 16 GiB in front; what is left is cut 20/80.
        let (root, home) = root_home_split(1000 * GIB, 512, 16).unwrap();
        let rest = root + home;
        assert_eq!(root, rest * 20 / 100);
        assert!(root > ROOT_MIN_GIB * 1024 && home > root * 3);
    }

    #[test]
    fn a_disk_under_128_gib_gives_the_system_forty_percent() {
        let (root, home) = root_home_split(100 * GIB, 512, 8).unwrap();
        assert_eq!(root, (root + home) * 40 / 100);
        assert!(root >= ROOT_MIN_GIB * 1024);
    }

    #[test]
    fn the_system_keeps_at_least_twenty_gib_and_a_tiny_disk_is_not_split() {
        // 128 GiB exactly: 20 % of the rest would be ~21 GiB, still above the floor.
        let (root, _) = root_home_split(128 * GIB, 512, 8).unwrap();
        assert!(root >= ROOT_MIN_GIB * 1024);
        // 40 GiB: after the swap too little is left to cut at all.
        assert_eq!(root_home_split(40 * GIB, 512, 8), None);
        assert_eq!(root_home_split(0, 512, 8), None);
    }

    #[tokio::test]
    async fn a_big_disk_gets_a_home_partition_with_its_own_btrfs() {
        let runner = FakeCommandRunner::new();
        runner.respond("blockdev", format!("{}\n", 1000 * GIB));
        let layout = plan("/dev/sda", RootFs::Btrfs, 16 * GIB);
        let parts = create_partitions(&runner, &layout).await.unwrap();
        assert_eq!(parts.home.as_deref(), Some("/dev/sda4"));
        let calls = runner.calls();
        assert!(
            calls
                .iter()
                .any(|(c, a)| c == "parted" && a.contains(&"home".to_string())),
            "{calls:?}"
        );
        format_partitions(&runner, &layout, &parts).await.unwrap();
        let calls = runner.calls();
        assert!(calls
            .iter()
            .any(|(c, a)| c == "mkfs.btrfs" && a.contains(&"/dev/sda4".to_string())));
        // root has no @home of its own, the home partition has
        let made: Vec<_> = calls
            .iter()
            .filter(|(c, a)| c == "btrfs" && a.contains(&"create".to_string()))
            .map(|(_, a)| a.last().cloned().unwrap_or_default())
            .collect();
        assert!(
            made.iter().filter(|p| p.ends_with("/@home")).count() == 1,
            "{made:?}"
        );
    }

    #[tokio::test]
    async fn format_partitions_btrfs_creates_subvolumes() {
        let runner = FakeCommandRunner::new();
        let layout = plan("/dev/sda", RootFs::Btrfs, 16 * 1024 * 1024 * 1024);
        let parts = Partitions {
            esp: "/dev/sda1".into(),
            swap: "/dev/sda2".into(),
            root: "/dev/sda3".into(),
            home: None,
        };

        format_partitions(&runner, &layout, &parts).await.unwrap();

        let calls = runner.calls();
        assert!(calls
            .iter()
            .any(|(cmd, args)| cmd == "mkfs.btrfs" && args.contains(&"/dev/sda3".to_string())));
        let subvol_calls: Vec<_> = calls
            .iter()
            .filter(|(cmd, args)| {
                cmd == "btrfs" && args.first().map(String::as_str) == Some("subvolume")
            })
            .collect();
        assert_eq!(subvol_calls.len(), BTRFS_SUBVOLUMES.len());
    }
}
