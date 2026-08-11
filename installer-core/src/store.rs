//! Wires the installed system to the distro's own Portage overlay + binhost
//! (the "portage store" this installer exists to sit in front of).

use std::path::Path;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub binhost_url: String,
    pub overlay_git_url: String,
    pub overlay_name: String,
}

/// Writes `/etc/portage/repos.conf/<overlay_name>.conf` and
/// `/etc/portage/binrepos.conf/<overlay_name>.conf` into the target root (pre-chroot,
/// path-prefixed), then syncs the overlay so the installed system can `emerge` immediately.
pub async fn configure(target_root: &Path, cfg: &StoreConfig) -> crate::Result<()> {
    let _ = (target_root, cfg);
    todo!("write repos.conf + binrepos.conf, chroot exec `emerge --sync --repo {{overlay_name}}`")
}

/// Lists atoms currently published on the binhost — used by `kernel::resolve` to check
/// which hardware-profile kernel packages actually exist before picking one.
pub async fn list_binhost_atoms(binhost_url: &str) -> crate::Result<Vec<String>> {
    let _ = binhost_url;
    todo!("GET {{binhost_url}}/Packages, parse CPV entries")
}
