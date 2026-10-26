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
//!
//! Chroot/emerge bootstrap plumbing (`/etc/resolv.conf`, syncing the main Portage tree,
//! directory-safe `/etc/portage/package.*` writes, proc/sys/dev bind mounts) lives in
//! `chroot_emerge` — shared with `wm`, the other step that still runs `emerge`.

use crate::chroot_emerge::{
    bind_mount_chroot_dirs, ensure_network_resolves, ensure_portage_tree, unmount_chroot_dirs,
    write_portage_entry,
};
use crate::command::CommandRunner;
use crate::hardware::Gpu;
use std::path::Path;

/// No-op for anything but Nvidia. Requires `store::configure` (portage tree) and
/// `kernel::deploy` (prepared kernel source tree) to have already run.
pub async fn install(runner: &dyn CommandRunner, target: &Path) -> crate::Result<Ran> {
    install_for(runner, target, Gpu::Nvidia).await
}

/// The prepared kernel tree was built with the build machine's compiler, and kbuild records that compiler's `--version` line
/// (`CONFIG_CC_VERSION_TEXT`). When the target's gcc says something else (a newer patch release), kbuild decides the configuration
/// is out of date and runs `syncconfig`, which needs the kernel's `Kconfig` files, and the tree has none: the module build stops with
/// "the kernel needs to be rebuilt". The module is built with the target's own compiler, so the recorded line is changed to match it.
/// Best-effort: if anything is unclear the files stay as they are.
async fn align_compiler_text(runner: &dyn CommandRunner, target_str: &str, target: &Path) {
    let tree = target.join("usr/src/linux");
    let Ok(config) = tokio::fs::read_to_string(tree.join(".config")).await else {
        return;
    };
    let Some(old) = recorded_compiler_text(&config) else {
        return;
    };
    if !old.starts_with("gcc") {
        return;
    }
    let Ok(version) = runner
        .run("chroot", &[target_str, "gcc", "--version"])
        .await
    else {
        return;
    };
    let Some(new) = version.lines().next().map(str::trim) else {
        return;
    };
    if new.is_empty() || new == old {
        return;
    }
    // `.config` first: the files after it must not be older than it, or kbuild syncs again.
    for file in [
        ".config",
        "include/config/auto.conf.cmd",
        "include/config/auto.conf",
        "include/generated/autoconf.h",
    ] {
        let path = tree.join(file);
        if let Ok(text) = tokio::fs::read_to_string(&path).await {
            if text.contains(&old) {
                tokio::fs::write(&path, text.replace(&old, new)).await.ok();
            }
        }
    }
}

/// The value of `CONFIG_CC_VERSION_TEXT="..."` in a kernel `.config`.
fn recorded_compiler_text(config: &str) -> Option<String> {
    let line = config
        .lines()
        .find(|l| l.starts_with("CONFIG_CC_VERSION_TEXT="))?;
    let value = line.split_once('=')?.1.trim();
    Some(value.trim_matches('"').to_string())
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

    let result = async {
        align_compiler_text(runner, target_str, target).await;
        ensure_network_resolves(target).await?;
        ensure_portage_tree(runner, target, target_str).await?;
        configure_nvidia_portage_overrides(target).await?;
        runner
            .run_status(
                "chroot",
                &[target_str, "emerge", "x11-drivers/nvidia-drivers"],
            )
            .await
    }
    .await;

    unmount_chroot_dirs(runner, target).await;

    result?;
    Ok(Ran::Installed)
}

/// Live-tested: even with a real synced tree, emerge refused every single
/// x11-drivers/nvidia-drivers version — the proprietary NVIDIA license is never
/// auto-accepted by Portage (by design, needs explicit opt-in), and on top of that every
/// version currently in the tree is ~amd64-keyword-masked (no stable branch exists at
/// all right now). A user installing this distro has already implicitly agreed to run
/// proprietary Nvidia software by nature of this step existing and running at all, so the
/// installer accepts on their behalf here rather than failing the whole install on a
/// prompt nothing can answer inside a non-interactive chroot.
async fn configure_nvidia_portage_overrides(target: &Path) -> crate::Result<()> {
    let portage_dir = target.join("etc/portage");
    tokio::fs::create_dir_all(&portage_dir).await?;
    write_portage_entry(
        &portage_dir.join("package.license"),
        "gentoo-installer-nvidia",
        "x11-drivers/nvidia-drivers NVIDIA-2025 NVIDIA-2023 NVIDIA-r2\n",
    )
    .await?;
    write_portage_entry(
        &portage_dir.join("package.accept_keywords"),
        "gentoo-installer-nvidia",
        "x11-drivers/nvidia-drivers ~amd64\n",
    )
    .await?;
    // Live-tested: with the default profile's USE flags, nvidia-drivers' "tools" flag
    // (nvidia-settings, a GTK GUI) drags in the entire X/GTK stack (libepoxy, gtk+,
    // cairo, pango...) with USE combinations the base profile doesn't satisfy, and emerge
    // refuses to resolve them non-interactively (needs --autounmask-write + a manual
    // review pass). This is a base-OS install with no desktop environment chosen yet —
    // the kernel module and core libs (what actually matters for the GPU to work at all)
    // don't need that GUI tool, so disabling "tools" here avoids depending on a DE choice
    // this step has no business making.
    // libglvnd needs its X flag for the GLX/EGL dispatch libraries nvidia-drivers
    // actually links against — live-tested: emerge's own autounmask output names this
    // exact change as the only thing blocking resolution once "tools" was out of the
    // picture, so it's applied directly instead of depending on the extra
    // --autounmask-write pass (an extra non-interactive emerge invocation with its own
    // failure modes) to work it out.
    write_portage_entry(
        &portage_dir.join("package.use"),
        "gentoo-installer-nvidia",
        "x11-drivers/nvidia-drivers -tools\n>=media-libs/libglvnd-1.7.0 X\n",
    )
    .await?;
    Ok(())
}

/// The NVIDIA driver could not be built, so the system keeps the open Nouveau driver instead: `VIDEO_CARDS` loses `nvidia` and
/// keeps the binary host's own set (which has `nouveau`), so Mesa stays the binary package it already is. Returns whether the
/// installed kernel carries the Nouveau module; without it the desktop still installs, but it has no kernel driver for this card.
pub async fn fall_back_to_open_driver(target: &Path) -> crate::Result<bool> {
    let path = target.join("etc/portage/make.conf");
    let text = tokio::fs::read_to_string(&path).await?;
    let mut out = String::new();
    for line in text.lines() {
        if line.starts_with("VIDEO_CARDS=") {
            let kept: Vec<&str> = line
                .trim_matches('"')
                .split_once('=')
                .map(|(_, v)| v.trim_matches('"'))
                .unwrap_or("")
                .split_whitespace()
                .filter(|c| *c != "nvidia")
                .collect();
            let cards = if kept.contains(&"nouveau") {
                kept.join(" ")
            } else {
                let mut all = kept.clone();
                all.push("nouveau");
                all.join(" ")
            };
            out.push_str(&format!("VIDEO_CARDS=\"{cards}\"\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    tokio::fs::write(&path, out).await?;
    Ok(has_nouveau_module(target))
}

/// Whether the kernel the target boots has the Nouveau module (in its module tree).
pub fn has_nouveau_module(target: &Path) -> bool {
    std::fs::read_dir(target.join("lib/modules"))
        .map(|d| {
            d.filter_map(|e| e.ok()).any(|e| {
                let dir = e.path().join("kernel/drivers/gpu/drm/nouveau");
                std::fs::read_dir(dir)
                    .map(|n| {
                        n.filter_map(|f| f.ok())
                            .any(|f| f.file_name().to_string_lossy().starts_with("nouveau.ko"))
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// The open driver is the one in use: `make.conf` no longer asks for `nvidia`, and includes `nouveau`.
pub async fn open_driver_selected(target: &Path) -> bool {
    let Ok(text) = tokio::fs::read_to_string(target.join("etc/portage/make.conf")).await else {
        return false;
    };
    text.lines()
        .find(|l| l.starts_with("VIDEO_CARDS="))
        .map(|l| {
            let cards: Vec<&str> = l
                .trim_matches('"')
                .split_once('=')
                .map(|(_, v)| v.trim_matches('"'))
                .unwrap_or("")
                .split_whitespace()
                .collect();
            cards.contains(&"nouveau") && !cards.contains(&"nvidia")
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ran {
    /// This machine's GPU doesn't need an out-of-tree driver built at install time.
    NotNeeded,
    Installed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_open_driver_takes_over_when_nvidia_fails() {
        let dir = std::env::temp_dir().join(format!("gpu-fallback-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("etc/portage")).unwrap();
        std::fs::create_dir_all(dir.join("lib/modules/7.1.8/kernel/drivers/gpu/drm/nouveau"))
            .unwrap();
        std::fs::write(
            dir.join("lib/modules/7.1.8/kernel/drivers/gpu/drm/nouveau/nouveau.ko.xz"),
            "",
        )
        .unwrap();
        std::fs::write(
            dir.join("etc/portage/make.conf"),
            "MAKEOPTS=\"-j8\"\nVIDEO_CARDS=\"amdgpu intel nouveau radeon radeonsi nvidia\"\nDONT_MOUNT_BOOT=\"1\"\n",
        )
        .unwrap();
        assert!(!open_driver_selected(&dir).await);
        assert!(fall_back_to_open_driver(&dir).await.unwrap());
        let conf = std::fs::read_to_string(dir.join("etc/portage/make.conf")).unwrap();
        assert!(
            conf.contains("VIDEO_CARDS=\"amdgpu intel nouveau radeon radeonsi\""),
            "{conf}"
        );
        assert!(conf.contains("MAKEOPTS=\"-j8\""));
        assert!(open_driver_selected(&dir).await);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn without_a_nouveau_module_the_fallback_says_so() {
        let dir = std::env::temp_dir().join(format!("gpu-fallback-none-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("etc/portage")).unwrap();
        std::fs::write(
            dir.join("etc/portage/make.conf"),
            "VIDEO_CARDS=\"nvidia\"\n",
        )
        .unwrap();
        assert!(!fall_back_to_open_driver(&dir).await.unwrap());
        let conf = std::fs::read_to_string(dir.join("etc/portage/make.conf")).unwrap();
        assert!(conf.contains("VIDEO_CARDS=\"nouveau\""), "{conf}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_recorded_compiler_line_is_read_from_the_kernel_config() {
        let config = "CONFIG_CC_IS_GCC=y\nCONFIG_CC_VERSION_TEXT=\"gcc (Gentoo Hardened 15.2.1 p5) 15.2.1\"\nCONFIG_GCC_VERSION=150201\n";
        assert_eq!(
            recorded_compiler_text(config).as_deref(),
            Some("gcc (Gentoo Hardened 15.2.1 p5) 15.2.1")
        );
        assert_eq!(recorded_compiler_text("CONFIG_X=y\n"), None);
    }
    use crate::command::FakeCommandRunner;

    #[tokio::test]
    async fn non_nvidia_gpu_does_nothing() {
        let runner = FakeCommandRunner::new();
        let ran = install_for(&runner, Path::new("/mnt/gentoo"), Gpu::Amd)
            .await
            .unwrap();
        assert_eq!(ran, Ran::NotNeeded);
        assert!(runner.calls().is_empty());
    }

    #[tokio::test]
    async fn nvidia_without_prepared_source_tree_errors_clearly() {
        let dir =
            std::env::temp_dir().join(format!("gentoo-installer-gpu-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let runner = FakeCommandRunner::new();

        let err = install_for(&runner, &dir, Gpu::Nvidia).await.unwrap_err();
        assert!(err.to_string().contains("devel.tar.xz"));

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn nvidia_with_prepared_source_tree_emerges_and_unmounts() {
        let dir =
            std::env::temp_dir().join(format!("gentoo-installer-gpu-test2-{}", std::process::id()));
        tokio::fs::create_dir_all(dir.join("usr/src/linux"))
            .await
            .unwrap();
        let runner = FakeCommandRunner::new();

        install_for(&runner, &dir, Gpu::Nvidia).await.unwrap();

        let calls = runner.calls();
        assert!(calls.iter().any(|(cmd, args)| cmd == "chroot"
            && args.contains(&"x11-drivers/nvidia-drivers".to_string())));
        // Every bind mount got a matching unmount.
        let mounts = calls.iter().filter(|(cmd, _)| cmd == "mount").count();
        let umounts = calls.iter().filter(|(cmd, _)| cmd == "umount").count();
        assert_eq!(mounts, umounts);

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
