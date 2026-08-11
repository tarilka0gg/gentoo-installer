//! Fetches and unpacks an official Gentoo stage3 tarball (amd64, openrc, multilib, non-hardened).

use std::path::Path;

pub const DEFAULT_MIRROR: &str = "https://distfiles.gentoo.org/releases/amd64/autobuilds";

#[derive(Debug, Clone)]
pub struct Stage3Source {
    /// e.g. "https://.../current-stage3-amd64-openrc/stage3-amd64-openrc-<date>.tar.xz"
    pub url: String,
    pub sha256: Option<String>,
}

/// Reads the mirror's `latest-stage3-amd64-openrc.txt` index to resolve the current
/// tarball filename, since Gentoo autobuilds are published under a date-stamped name.
pub async fn resolve_latest() -> crate::Result<Stage3Source> {
    todo!("fetch {DEFAULT_MIRROR}/latest-stage3-amd64-openrc.txt, parse filename + sha256")
}

pub async fn download(source: &Stage3Source, dest: &Path) -> crate::Result<()> {
    let _ = (source, dest);
    todo!("stream download with progress reporting, verify sha256")
}

/// Unpacks the tarball into `root` (typically the mounted target `@` subvolume),
/// preserving ownership/xattrs (`tar --xattrs -p`).
pub async fn unpack(tarball: &Path, root: &Path) -> crate::Result<()> {
    let _ = (tarball, root);
    todo!("tar -xpf {{tarball}} -C {{root}} --xattrs-include='*.*' --numeric-owner")
}
