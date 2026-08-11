//! Automatic partitioning: ESP + swap + root, btrfs (default, with subvolumes) or ext4.

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

/// Runs the actual `parted`/`mkfs.*`/subvolume-create sequence against `layout.disk`.
/// Destructive — callers must have already confirmed with the user.
pub async fn apply(layout: &Layout) -> crate::Result<()> {
    let _ = layout;
    todo!("parted mklabel gpt; mkpart ESP/swap/root; mkfs.vfat/mkfs.btrfs|mkfs.ext4; btrfs subvolume create for BTRFS_SUBVOLUMES")
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
}
