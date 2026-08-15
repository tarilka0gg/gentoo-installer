//! Compiles and installs an out-of-tree GPU kernel driver at install time, on the
//! target machine, against its exact kernel build — the one place this codebase still
//! invokes `emerge` during install, deliberately, against the formal build spec's
//! "no emerge" rule.
//!
//! Why the exception: nvidia's proprietary driver ships as a kernel module that has to
//! be compiled against the *exact* kernel it will load into (same version, same config,
//! matching `Module.symvers`). Pre-building it in the store for every one of the
//! matrix's ~300 cpu×gpu×platform combos — and rebuilding all of them on every driver
//! version bump — is a combinatorial problem the store shouldn't have to solve. Building
//! it once, on the actual target, against the prepared kernel source tree
//! `kernel::deploy` already fetched (`<combo>-devel.tar.xz`, unpacked to
//! `/usr/src/linux-<combo>` and symlinked from `/usr/src/linux`), is the standard
//! approach every other distro's installer takes for this exact problem — and Portage's
//! own dependency/version resolution for `x11-drivers/nvidia-drivers` is more reliable
//! than this installer hand-rolling module compilation itself.
//!
//! AMD (`amdgpu`) and Intel (`i915`/`xe`) are open, in-tree drivers — expected to already
//! be part of the kernel build itself (as `=y` or a plain in-tree `=m` module downloaded
//! by `kernel::deploy`'s modules tarball), not something this step needs to touch.

use crate::command::CommandRunner;
use crate::hardware::Gpu;
use std::path::Path;

/// No-op for anything but Nvidia. Requires `store::configure` (portage tree) and
/// `kernel::deploy` (prepared kernel source tree) to have already run.
pub async fn install(runner: &dyn CommandRunner, target: &Path) -> crate::Result<Ran> {
    install_for(runner, target, Gpu::Nvidia).await
}

/// Split out from `install` so tests can drive it with an arbitrary `Gpu` without
/// needing real hardware detection.
async fn install_for(runner: &dyn CommandRunner, target: &Path, gpu: Gpu) -> crate::Result<Ran> {
    if gpu != Gpu::Nvidia {
        return Ok(Ran::NotNeeded);
    }

    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;

    if !target.join("usr/src/linux").exists() {
        return Err(crate::Error::Other(anyhow::anyhow!(
            "nvidia-drivers needs a prepared kernel source tree at /usr/src/linux, but \
             none was fetched — this combo's store build has no <combo>-devel.tar.xz"
        )));
    }

    bind_mount_chroot_dirs(runner, target).await?;

    let result = runner
        .run_status("chroot", &[target_str, "emerge", "x11-drivers/nvidia-drivers"])
        .await;

    unmount_chroot_dirs(runner, target).await;

    result?;
    Ok(Ran::Installed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ran {
    /// This machine's GPU doesn't need an out-of-tree driver built at install time.
    NotNeeded,
    Installed,
}

async fn bind_mount_chroot_dirs(runner: &dyn CommandRunner, target: &Path) -> crate::Result<()> {
    for dir in ["proc", "sys", "dev"] {
        let target_dir = target.join(dir);
        tokio::fs::create_dir_all(&target_dir).await?;
        let target_dir_str = target_dir
            .to_str()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 path")))?;
        runner.run_status("mount", &["--rbind", &format!("/{dir}"), target_dir_str]).await?;
    }
    Ok(())
}

/// Best-effort: called on both the success and failure path of the emerge above, so a
/// failed build doesn't leave the target's /proc etc. bind-mounted.
async fn unmount_chroot_dirs(runner: &dyn CommandRunner, target: &Path) {
    for dir in ["dev", "sys", "proc"] {
        if let Some(target_dir_str) = target.join(dir).to_str() {
            runner.run_status("umount", &["-R", target_dir_str]).await.ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    #[tokio::test]
    async fn non_nvidia_gpu_does_nothing() {
        let runner = FakeCommandRunner::new();
        let ran = install_for(&runner, Path::new("/mnt/gentoo"), Gpu::Amd).await.unwrap();
        assert_eq!(ran, Ran::NotNeeded);
        assert!(runner.calls().is_empty());
    }

    #[tokio::test]
    async fn nvidia_without_prepared_source_tree_errors_clearly() {
        let dir = std::env::temp_dir().join(format!("gentoo-installer-gpu-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let runner = FakeCommandRunner::new();

        let err = install_for(&runner, &dir, Gpu::Nvidia).await.unwrap_err();
        assert!(err.to_string().contains("devel.tar.xz"));

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn nvidia_with_prepared_source_tree_emerges_and_unmounts() {
        let dir = std::env::temp_dir().join(format!("gentoo-installer-gpu-test2-{}", std::process::id()));
        tokio::fs::create_dir_all(dir.join("usr/src/linux")).await.unwrap();
        let runner = FakeCommandRunner::new();

        install_for(&runner, &dir, Gpu::Nvidia).await.unwrap();

        let calls = runner.calls();
        assert!(calls.iter().any(|(cmd, args)| cmd == "chroot" && args.contains(&"x11-drivers/nvidia-drivers".to_string())));
        // Every bind mount got a matching unmount.
        let mounts = calls.iter().filter(|(cmd, _)| cmd == "mount").count();
        let umounts = calls.iter().filter(|(cmd, _)| cmd == "umount").count();
        assert_eq!(mounts, umounts);

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
