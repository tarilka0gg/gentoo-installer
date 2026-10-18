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

/// Where on the disk the system goes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Placement {
    /// The whole disk, with a fresh partition table: everything on it is lost. The default.
    #[default]
    WholeDisk,
    /// Into a free region of a disk that keeps its partitions (GPT): nothing outside the region is touched.
    FreeSpace {
        /// The region, in MiB from the start of the disk (`end_mib` is the first MiB that is not ours).
        start_mib: u64,
        end_mib: u64,
        /// The EFI system partition that is already on the disk and is shared with the other systems; `None` makes one
        /// in the region.
        esp: Option<String>,
        /// The partition numbers the new partitions get (the lowest unused ones, chosen when the plan is made):
        /// ESP (unused when `esp` is `Some`), swap, root and home (unused when there is no separate home).
        esp_n: u32,
        swap_n: u32,
        root_n: u32,
        home_n: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub disk: String,
    /// Whole disk (the default) or a free region of it.
    #[serde(default)]
    pub placement: Placement,
    pub esp_size_mib: u64,
    pub swap_gib: u64,
    pub root_fs: RootFs,
    /// `/home` on a partition of its own when the disk is big enough (see [`root_home_split`]).
    #[serde(default = "default_true")]
    pub separate_home: bool,
    /// A 1 MiB BIOS boot partition at the very end of the disk, which Limine needs to boot a GPT disk through legacy BIOS (it keeps
    /// its second stage there). Made when the machine itself booted through BIOS; last, so that the other partitions keep their numbers.
    #[serde(default)]
    pub bios_boot: bool,
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
        placement: Placement::WholeDisk,
        esp_size_mib: ESP_SIZE_MIB,
        swap_gib: swap_gib.clamp(SWAP_MIN_GIB, SWAP_MAX_GIB),
        root_fs,
        separate_home: true,
        bios_boot: !crate::bootloader::firmware_is_uefi(),
    }
}

/// A layout for `region` of a disk that keeps its partitions: `size_gib` of it (all of it when `None`), from its start. Errors say why
/// the disk cannot take it: not GPT, or too little room.
pub fn layout_in_free_space(
    inspection: &crate::disk::Inspection,
    disk: &str,
    root_fs: RootFs,
    swap_gib: u64,
    region: crate::disk::FreeRegion,
    size_gib: Option<u64>,
) -> crate::Result<Layout> {
    let fail = |msg: String| Err(crate::Error::Other(anyhow::anyhow!(msg)));
    if inspection.table != "gpt" {
        return fail(format!(
            "{disk} has a {} partition table; installing next to other systems needs GPT (use the whole disk, or convert it first)",
            if inspection.table.is_empty() { "no" } else { inspection.table.as_str() }
        ));
    }
    let esp = inspection.esp().map(|p| p.device.clone());
    let wanted = size_gib.map_or(region.size_mib(), |g| (g * 1024).min(region.size_mib()));
    let need = min_free_mib(esp.is_some());
    if wanted < need {
        return fail(format!(
            "{} MiB of free space is too little: the system needs at least {need} MiB",
            wanted
        ));
    }
    let existing: Vec<u32> = inspection.partitions.iter().map(|p| p.number).collect();
    Ok(plan_free_space(
        disk,
        root_fs,
        swap_gib,
        (region.start_mib, region.start_mib + wanted),
        &existing,
        esp,
    ))
}

/// [`layout_in_free_space`] for the largest free region of `disk`, read now.
pub async fn plan_next_to_existing(
    runner: &dyn CommandRunner,
    disk: &str,
    root_fs: RootFs,
    swap_gib: u64,
    size_gib: Option<u64>,
) -> crate::Result<Layout> {
    let inspection = crate::disk::inspect(runner, disk).await?;
    let Some(region) = inspection.largest_free() else {
        return Err(crate::Error::Other(anyhow::anyhow!(
            "{disk} has no free space"
        )));
    };
    layout_in_free_space(&inspection, disk, root_fs, swap_gib, region, size_gib)
}

/// The smallest free region the installer offers to install into: ESP (when none is shared) + the minimal swap + a usable root.
pub fn min_free_mib(esp_exists: bool) -> u64 {
    (if esp_exists { 0 } else { ESP_SIZE_MIB }) + SWAP_MIN_GIB * 1024 + ROOT_MIN_GIB * 1024 + 2
}

/// A layout that puts the system in `start_mib..end_mib` of `disk`, next to what is already there. `existing` are the numbers of the
/// partitions that are on the disk; the new ones take the lowest numbers not in it. `esp` is the shared EFI system partition, when
/// the disk has one.
pub fn plan_free_space(
    disk: &str,
    root_fs: RootFs,
    swap_gib: u64,
    (start_mib, end_mib): (u64, u64),
    existing: &[u32],
    esp: Option<String>,
) -> Layout {
    let mut free_numbers = (1u32..).filter(|n| !existing.contains(n));
    let mut next = || free_numbers.next().unwrap_or(1);
    let (esp_n, swap_n, root_n, home_n) = (next(), next(), next(), next());
    Layout {
        placement: Placement::FreeSpace {
            start_mib,
            end_mib,
            esp,
            esp_n,
            swap_n,
            root_n,
            home_n,
        },
        // next to other systems the layout is only the three partitions in the free region (a BIOS boot partition is not made there)
        bios_boot: false,
        ..plan_with_swap(disk, root_fs, swap_gib)
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
pub fn part_device(disk: &str, index: u32) -> String {
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
    if let Placement::FreeSpace {
        esp,
        esp_n,
        swap_n,
        root_n,
        home_n,
        ..
    } = &layout.placement
    {
        let home = part_device(&layout.disk, *home_n);
        let has_home = layout.separate_home && std::path::Path::new(&home).exists();
        return Partitions {
            esp: esp
                .clone()
                .unwrap_or_else(|| part_device(&layout.disk, *esp_n)),
            swap: part_device(&layout.disk, *swap_n),
            root: part_device(&layout.disk, *root_n),
            home: has_home.then_some(home),
        };
    }
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
    if let Placement::FreeSpace { .. } = &layout.placement {
        return create_in_free_space(runner, layout).await;
    }
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
    // The last of `/` and `/home` ends 2 MiB before the end of the disk when the BIOS boot partition goes after it.
    let tail = if layout.bios_boot { "-2MiB" } else { "100%" };
    let root_end = match split {
        Some((root_mib, _)) => format!("{}MiB", swap_end_mib + root_mib),
        None => tail.to_string(),
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
                &["--script", disk, "mkpart", "home", &root_end, tail],
            )
            .await?;
    }
    if layout.bios_boot {
        // Limine's `bios-install` finds it by its type (the `bios_grub` flag) on a GPT disk; after `/home` or `/` so that the numbers
        // of the others stay: 4 without `/home`, 5 with it.
        let n = if split.is_some() { "5" } else { "4" };
        runner
            .run_status(
                "parted",
                &["--script", disk, "mkpart", "biosboot", "-2MiB", "-1MiB"],
            )
            .await?;
        runner
            .run_status("parted", &["--script", disk, "set", n, "bios_grub", "on"])
            .await?;
    }

    // Re-read the partition table so the new device nodes exist before mkfs.
    runner.run_status("partprobe", &[disk]).await.ok();

    Ok(Partitions {
        home: split.map(|_| part_device(disk, 4)),
        ..partitions_for(layout)
    })
}

/// Makes the partitions inside the free region of [`Placement::FreeSpace`] with `sgdisk`, at the numbers the plan chose, and
/// touches nothing else on the disk.
async fn create_in_free_space(
    runner: &dyn CommandRunner,
    layout: &Layout,
) -> crate::Result<Partitions> {
    let Placement::FreeSpace {
        start_mib,
        end_mib,
        esp,
        esp_n,
        swap_n,
        root_n,
        home_n,
    } = &layout.placement
    else {
        unreachable!("create_in_free_space is only called for FreeSpace");
    };
    let disk = &layout.disk;
    let esp_mib = if esp.is_some() {
        0
    } else {
        layout.esp_size_mib
    };
    let region_mib = end_mib.saturating_sub(*start_mib);
    let split = if layout.separate_home {
        // The split works on a disk's bytes with a MiB of alignment gap in front and one behind: the region has the same shape.
        root_home_split(region_mib * 1024 * 1024, esp_mib, layout.swap_gib)
    } else {
        None
    };
    // (number, start MiB, size MiB, type code, name)
    let mut cursor = *start_mib;
    let mut plan: Vec<(u32, u64, u64, &str, &str)> = Vec::new();
    if esp.is_none() {
        plan.push((*esp_n, cursor, layout.esp_size_mib, "ef00", "ESP"));
        cursor += layout.esp_size_mib;
    }
    plan.push((*swap_n, cursor, layout.swap_gib * 1024, "8200", "swap"));
    cursor += layout.swap_gib * 1024;
    let tail = end_mib.saturating_sub(1); // leave the last MiB, as the whole-disk layout does
    match split {
        Some((root_mib, _)) => {
            plan.push((*root_n, cursor, root_mib, "8300", "root"));
            cursor += root_mib;
            plan.push((*home_n, cursor, tail.saturating_sub(cursor), "8300", "home"));
        }
        None => plan.push((*root_n, cursor, tail.saturating_sub(cursor), "8300", "root")),
    }
    for (number, start, size, code, name) in &plan {
        runner
            .run_status(
                "sgdisk",
                &[
                    "-n",
                    &format!("{number}:{start}M:+{size}M"),
                    "-t",
                    &format!("{number}:{code}"),
                    "-c",
                    &format!("{number}:{name}"),
                    disk,
                ],
            )
            .await?;
    }
    runner.run_status("partprobe", &[disk]).await.ok();
    Ok(Partitions {
        home: split.map(|_| part_device(disk, *home_n)),
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
    // A shared EFI system partition holds the other systems' boot files: it is mounted, never formatted.
    let esp_is_shared = matches!(&layout.placement, Placement::FreeSpace { esp: Some(_), .. });
    if !esp_is_shared {
        runner
            .run_status("mkfs.vfat", &["-F32", "-n", "ESP", &parts.esp])
            .await?;
    }
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
    async fn free_space_gets_new_partitions_at_unused_numbers_and_nothing_else_is_touched() {
        let runner = FakeCommandRunner::new();
        // A disk with partitions 1 (the shared ESP), 2 and 3 in use; 200 GiB free at the end.
        let layout = plan_free_space(
            "/dev/sda",
            RootFs::Btrfs,
            8,
            (300 * 1024, 500 * 1024),
            &[1, 2, 3],
            Some("/dev/sda1".into()),
        );
        let parts = create_partitions(&runner, &layout).await.unwrap();
        assert_eq!(parts.esp, "/dev/sda1", "the shared ESP is reused");
        assert_eq!(parts.swap, "/dev/sda5");
        assert_eq!(parts.root, "/dev/sda6");
        let calls = runner.calls();
        assert!(
            calls
                .iter()
                .all(|(c, _)| c == "sgdisk" || c == "partprobe" || c == "blockdev"),
            "no mklabel, no wipe: {calls:?}"
        );
        // swap 8 GiB first, then root (20 % of the rest), then home
        runner.assert_call(
            0,
            "sgdisk",
            &[
                "-n",
                "5:307200M:+8192M",
                "-t",
                "5:8200",
                "-c",
                "5:swap",
                "/dev/sda",
            ],
        );
        let root = calls
            .iter()
            .find(|(_, a)| a.iter().any(|x| x.starts_with("6:3")))
            .unwrap();
        assert!(root.1[1].starts_with("6:315392M:+"), "{root:?}");
        let home = calls
            .iter()
            .find(|(_, a)| a.iter().any(|x| x.starts_with("7:")))
            .unwrap();
        assert!(home.1[1].starts_with("7:"), "{home:?}");
    }

    #[tokio::test]
    async fn a_new_esp_is_made_only_when_none_is_shared() {
        let runner = FakeCommandRunner::new();
        let layout = plan_free_space("/dev/vda", RootFs::Btrfs, 8, (1024, 60 * 1024), &[], None);
        create_partitions(&runner, &layout).await.unwrap();
        runner.assert_call(
            0,
            "sgdisk",
            &[
                "-n",
                "1:1024M:+512M",
                "-t",
                "1:ef00",
                "-c",
                "1:ESP",
                "/dev/vda",
            ],
        );
        // a shared one is never formatted
        let shared = plan_free_space(
            "/dev/vda",
            RootFs::Btrfs,
            8,
            (1024, 60 * 1024),
            &[1],
            Some("/dev/vda1".into()),
        );
        let runner = FakeCommandRunner::new();
        let parts = partitions_for(&shared);
        format_partitions(&runner, &shared, &parts).await.unwrap();
        assert!(!runner
            .calls()
            .iter()
            .any(|(c, a)| c == "mkfs.vfat" || a.contains(&"/dev/vda1".to_string())));
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

    fn parted_calls(runner: &FakeCommandRunner) -> Vec<Vec<String>> {
        runner.calls().into_iter().filter(|(c, _)| c == "parted").map(|(_, a)| a).collect()
    }

    #[tokio::test]
    async fn a_bios_machine_gets_a_bios_boot_partition_after_the_others_so_they_keep_their_numbers() {
        // Limine cannot boot a GPT disk through legacy BIOS without one ("no BIOS boot partition specified or detected").
        let runner = FakeCommandRunner::new(); // no size known: one root partition
        let mut layout = plan("/dev/sda", RootFs::Btrfs, 16 * GIB);
        layout.bios_boot = true;
        let parts = create_partitions(&runner, &layout).await.unwrap();
        assert_eq!((parts.esp.as_str(), parts.swap.as_str(), parts.root.as_str()), ("/dev/sda1", "/dev/sda2", "/dev/sda3"));
        let calls = parted_calls(&runner);
        let root = calls.iter().find(|a| a.contains(&"root".to_string())).unwrap();
        assert_eq!(root[root.len() - 1], "-2MiB", "root stops short of the end: {root:?}");
        let n = calls.len();
        assert_eq!(calls[n - 2], ["--script", "/dev/sda", "mkpart", "biosboot", "-2MiB", "-1MiB"]);
        assert_eq!(calls[n - 1], ["--script", "/dev/sda", "set", "4", "bios_grub", "on"]);
    }

    #[tokio::test]
    async fn with_a_home_partition_the_bios_boot_partition_is_the_fifth() {
        let runner = FakeCommandRunner::new();
        runner.respond("blockdev", format!("{}\n", 1000 * GIB));
        let mut layout = plan("/dev/sda", RootFs::Btrfs, 16 * GIB);
        layout.bios_boot = true;
        let parts = create_partitions(&runner, &layout).await.unwrap();
        assert_eq!(parts.home.as_deref(), Some("/dev/sda4"));
        let calls = parted_calls(&runner);
        let home = calls.iter().find(|a| a.contains(&"home".to_string())).unwrap();
        assert_eq!(home[home.len() - 1], "-2MiB");
        assert_eq!(calls[calls.len() - 1], ["--script", "/dev/sda", "set", "5", "bios_grub", "on"]);
    }

    #[tokio::test]
    async fn a_uefi_machine_gets_no_bios_boot_partition_and_the_old_ends() {
        let runner = FakeCommandRunner::new();
        let mut layout = plan("/dev/sda", RootFs::Btrfs, 16 * GIB);
        layout.bios_boot = false;
        create_partitions(&runner, &layout).await.unwrap();
        let calls = parted_calls(&runner);
        assert!(!calls.iter().any(|a| a.iter().any(|x| x == "bios_grub" || x == "biosboot")));
        let root = calls.iter().find(|a| a.contains(&"root".to_string())).unwrap();
        assert_eq!(root[root.len() - 1], "100%");
    }

    #[test]
    fn a_layout_saved_before_the_field_existed_loads_without_one() {
        let mut layout = plan("/dev/sda", RootFs::Btrfs, 16 * GIB);
        layout.bios_boot = true;
        let mut json: serde_json::Value = serde_json::to_value(&layout).unwrap();
        json.as_object_mut().unwrap().remove("bios_boot");
        let old: Layout = serde_json::from_value(json).unwrap();
        assert!(!old.bios_boot);
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
