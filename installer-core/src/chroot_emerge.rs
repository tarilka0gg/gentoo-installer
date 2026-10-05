//! Shared chroot/emerge bootstrap infrastructure — used by every step that still runs
//! `emerge` against the target during install (currently `gpu_driver` and `wm`), against
//! the formal build spec's "no emerge" rule. Each caller documents its own reason for the
//! exception; this module just solves the plumbing common to all of them: making a bare
//! stage3 chroot actually able to resolve DNS, have a synced Portage tree, and accept
//! per-package portage overrides, without duplicating that logic per caller.

use crate::command::CommandRunner;
use std::path::Path;

/// `chroot`'d commands see the target's own `/etc/resolv.conf`, not the live installer
/// environment's — live-tested against a fresh stage3, where it's either absent or a
/// stub, so any network operation inside the chroot fails DNS resolution before it gets
/// anywhere near a real error. Copying (not bind-mounting) the host's resolver config in
/// is standard practice for this exact chroot-needs-network situation; whatever the
/// installed system's own network stack (NetworkManager, systemd-resolved, dhcpcd...)
/// writes there on first boot overwrites this without any cleanup needed here.
pub(crate) async fn ensure_network_resolves(target: &Path) -> crate::Result<()> {
    let dest = target.join("etc/resolv.conf");
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy("/etc/resolv.conf", dest).await?;
    Ok(())
}

/// `store::configure` deliberately only clones this distro's own overlay and defers
/// syncing the main `gentoo` tree to the installed system's first boot (see that
/// module's doc comment) — but packages like `x11-drivers/nvidia-drivers` live in the
/// main tree, not the overlay, so any install-time `emerge` needs that tree to exist
/// *now*. Live-tested: without it, `make.profile`'s (correctly stage3-provided) symlink
/// into `var/db/repos/gentoo/profiles/...` points nowhere, and emerge refuses to do
/// anything but --sync/--info/--help/--search/--version.
/// `emerge-webrsync` (a single signed snapshot tarball) is used instead of `emerge --sync`
/// (an incremental rsync/git tree walk) because bootstrapping from nothing is exactly the
/// case the webrsync snapshot exists for — much less network and CPU than a full rsync of
/// the entire tree's history-aware sync protocol for a first-ever sync.
pub(crate) async fn ensure_portage_tree(
    runner: &dyn CommandRunner,
    target: &Path,
    target_str: &str,
) -> crate::Result<()> {
    if target.join("var/db/repos/gentoo/profiles").exists() {
        return Ok(());
    }
    runner
        .run_status("chroot", &[target_str, "emerge-webrsync"])
        .await?;
    Ok(())
}

/// `package.license`/`package.accept_keywords`/etc. under `/etc/portage` are each
/// allowed by Portage to be either a single file *or* a directory of per-topic files —
/// live-tested against a real stage3, which ships them as (empty) directories so
/// overlays/profiles/the installer can each drop their own file in without clobbering
/// anyone else's. Writing straight to the path as if it were always a plain file fails
/// with "Is a directory" the moment stage3 actually does this, which it does.
pub(crate) async fn write_portage_entry(
    path: &Path,
    filename: &str,
    content: &str,
) -> crate::Result<()> {
    let target_file = if tokio::fs::metadata(path)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        path.join(filename)
    } else {
        path.to_path_buf()
    };
    tokio::fs::write(target_file, content).await?;
    Ok(())
}

pub(crate) async fn bind_mount_chroot_dirs(
    runner: &dyn CommandRunner,
    target: &Path,
) -> crate::Result<()> {
    for dir in ["proc", "sys", "dev"] {
        let target_dir = target.join(dir);
        tokio::fs::create_dir_all(&target_dir).await?;
        let target_dir_str = target_dir
            .to_str()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 path")))?;
        runner
            .run_status("mount", &["--rbind", &format!("/{dir}"), target_dir_str])
            .await?;
    }
    Ok(())
}

/// Best-effort: meant to be called on both the success and failure path of whatever ran
/// between the matching `bind_mount_chroot_dirs`, so a failed build doesn't leave the
/// target's /proc etc. bind-mounted.
pub(crate) async fn unmount_chroot_dirs(runner: &dyn CommandRunner, target: &Path) {
    for dir in ["dev", "sys", "proc"] {
        if let Some(target_dir_str) = target.join(dir).to_str() {
            runner
                .run_status("umount", &["-R", target_dir_str])
                .await
                .ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    #[tokio::test]
    async fn bind_mount_and_unmount_counts_match() {
        let dir = std::env::temp_dir().join(format!(
            "gentoo-installer-chroot-emerge-test-{}",
            std::process::id()
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let runner = FakeCommandRunner::new();

        bind_mount_chroot_dirs(&runner, &dir).await.unwrap();
        unmount_chroot_dirs(&runner, &dir).await;

        let calls = runner.calls();
        let mounts = calls.iter().filter(|(cmd, _)| cmd == "mount").count();
        let umounts = calls.iter().filter(|(cmd, _)| cmd == "umount").count();
        assert_eq!(mounts, 3);
        assert_eq!(umounts, 3);

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn write_portage_entry_targets_file_inside_existing_directory() {
        let dir = std::env::temp_dir().join(format!(
            "gentoo-installer-chroot-emerge-test2-{}",
            std::process::id()
        ));
        let portage_path = dir.join("package.license");
        tokio::fs::create_dir_all(&portage_path).await.unwrap();

        write_portage_entry(&portage_path, "myfile", "some content\n")
            .await
            .unwrap();

        let written = tokio::fs::read_to_string(portage_path.join("myfile"))
            .await
            .unwrap();
        assert_eq!(written, "some content\n");

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn write_portage_entry_targets_path_directly_when_not_a_directory() {
        let dir = std::env::temp_dir().join(format!(
            "gentoo-installer-chroot-emerge-test3-{}",
            std::process::id()
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let portage_path = dir.join("package.use");

        write_portage_entry(&portage_path, "myfile", "some content\n")
            .await
            .unwrap();

        let written = tokio::fs::read_to_string(&portage_path).await.unwrap();
        assert_eq!(written, "some content\n");

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
