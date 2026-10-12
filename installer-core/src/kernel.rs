//! Picks a precompiled kernel binary from the store that matches the detected (or
//! manually overridden) hardware profile. Only a popularity-ranked subset of the full
//! cpu×gpu×platform matrix is actually built (304 of 378 as of the last matrix run — see
//! `hardware::Profile::candidates`), so this degrades through progressively more generic
//! combos rather than requiring an exact match.
//!
//! Deployment is a direct file copy (`deploy`), not `emerge` — per spec, the installer
//! never invokes emerge; everything is pre-resolved. This also matches the store's actual
//! current shape: kernel builds are raw `<combo>.vmlinuz` + module tree output from
//! `matrix-build.sh`, not Portage binpkgs yet. `atom` is kept as a stable match key /
//! display string in the `sys-kernel/<base>-bin-<combo>` shape (so `store::list_binhost_atoms`
//! and its tests don't need to change), not a literal ebuild atom this code ever emerges.

use crate::command::CommandRunner;
use crate::hardware::Profile;
use crate::http;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct KernelPackage {
    /// Match key / display string, e.g. "sys-kernel/mykernel-bin-intel-raptorlake-nvidia-laptop".
    pub atom: String,
    /// The bare combo (e.g. "intel-raptorlake-nvidia-laptop") — what `deploy` needs to
    /// build the actual fetch URL, without re-parsing it back out of `atom`.
    pub combo: String,
    /// Which candidate in the degrade chain matched — 0 means an exact match, higher
    /// means the profile's platform/cpu/gpu got generalized to find a build that
    /// actually exists in the store.
    pub degraded_by: usize,
}

/// The URL of a file of the kernel store. A GitHub release has no directories, its assets are flat
/// (`<base>/releases/download/<tag>/<file>`); any other store is a web directory with the files under `kernels/`.
pub fn kernel_asset_url(base: &str, file: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.contains("/releases/download/") {
        format!("{base}/{file}")
    } else {
        format!("{base}/kernels/{file}")
    }
}

pub fn resolve(
    base_name: &str,
    profile: &Profile,
    available_atoms: &[String],
) -> crate::Result<KernelPackage> {
    for (i, combo) in profile.candidates().into_iter().enumerate() {
        let atom = format!("sys-kernel/{base_name}-bin-{combo}");
        if available_atoms.iter().any(|a| a == &atom) {
            return Ok(KernelPackage {
                atom,
                combo,
                degraded_by: i,
            });
        }
    }
    Err(crate::Error::NoKernelProfile(profile.combo()))
}

/// Fetches the matched kernel's vmlinuz and module tree from the store and installs them
/// under `target`. Assumes the store publishes each build at
/// `<binhost_url>/kernels/<combo>.vmlinuz` and `<binhost_url>/kernels/<combo>-modules.tar.xz`
/// — a convention, not yet a settled publishing layout (the store currently only has raw
/// local build output, see `~/kernel-releases/`), so this is the first thing to update
/// once that's decided for real.
pub async fn deploy(
    runner: &dyn CommandRunner,
    binhost_url: &str,
    combo: &str,
    target: &Path,
) -> crate::Result<()> {
    let boot_dir = target.join("boot");
    tokio::fs::create_dir_all(&boot_dir).await?;
    let vmlinuz_dest = boot_dir.join(format!("vmlinuz-{combo}"));
    http::download_to_file(
        &kernel_asset_url(binhost_url, &format!("{combo}.vmlinuz")),
        &vmlinuz_dest,
    )
    .await?;

    let modules_tarball =
        std::env::temp_dir().join(format!("gentoo-installer-{combo}-modules.tar.xz"));
    http::download_to_file(
        &kernel_asset_url(binhost_url, &format!("{combo}-modules.tar.xz")),
        &modules_tarball,
    )
    .await?;

    let modules_dir = target.join("lib/modules");
    tokio::fs::create_dir_all(&modules_dir).await?;
    let modules_tarball_str = modules_tarball
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 tarball path")))?;
    let modules_dir_str = modules_dir
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 modules dir path")))?;
    runner
        .run_status("tar", &["-xpf", modules_tarball_str, "-C", modules_dir_str])
        .await?;
    tokio::fs::remove_file(&modules_tarball).await.ok();

    // Optional: a prepared kernel source tree (headers + Module.symvers, i.e. the
    // output of `make modules_prepare` against this exact build) — not fetched for
    // every combo, only ones that need an out-of-tree module compiled against them at
    // install time (see `gpu_driver::install`). A missing file here just means this
    // combo doesn't need one; not an install-blocking error.
    let devel_url = kernel_asset_url(binhost_url, &format!("{combo}-devel.tar.xz"));
    let devel_tarball = std::env::temp_dir().join(format!("gentoo-installer-{combo}-devel.tar.xz"));
    if http::download_to_file(&devel_url, &devel_tarball)
        .await
        .is_ok()
    {
        let src_dir = target.join(format!("usr/src/linux-{combo}"));
        tokio::fs::create_dir_all(&src_dir).await?;
        let src_dir_str = src_dir
            .to_str()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 kernel src dir path")))?;
        let devel_tarball_str = devel_tarball
            .to_str()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 devel tarball path")))?;
        runner
            .run_status("tar", &["-xpf", devel_tarball_str, "-C", src_dir_str])
            .await?;
        tokio::fs::remove_file(&devel_tarball).await.ok();

        let linux_symlink = target.join("usr/src/linux");
        tokio::fs::remove_file(&linux_symlink).await.ok();
        tokio::fs::symlink(format!("linux-{combo}"), &linux_symlink).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{CpuArch, Gpu, Platform};

    #[test]
    fn a_github_release_is_flat_and_any_other_store_has_a_kernels_directory() {
        assert_eq!(
            kernel_asset_url(
                "https://github.com/o/r/releases/download/kernels/",
                "a.vmlinuz"
            ),
            "https://github.com/o/r/releases/download/kernels/a.vmlinuz"
        );
        assert_eq!(
            kernel_asset_url("http://10.0.2.2:8123/", "a.vmlinuz"),
            "http://10.0.2.2:8123/kernels/a.vmlinuz"
        );
    }

    #[test]
    fn exact_match_wins_when_present() {
        let profile = Profile {
            cpu: CpuArch::IntelRaptorlake,
            gpu: Gpu::Nvidia,
            platform: Platform::Laptop,
            ram_bytes: 0,
        };
        let atoms = vec!["sys-kernel/mykernel-bin-intel-raptorlake-nvidia-laptop".to_string()];
        let pkg = resolve("mykernel", &profile, &atoms).unwrap();
        assert_eq!(pkg.degraded_by, 0);
        assert_eq!(pkg.combo, "intel-raptorlake-nvidia-laptop");
    }

    #[test]
    fn falls_back_when_exact_combo_missing() {
        let profile = Profile {
            cpu: CpuArch::AmdZnver3,
            gpu: Gpu::Intel,
            platform: Platform::Handheld,
            ram_bytes: 0,
        };
        // Only the laptop-degraded combo exists in the store.
        let atoms = vec!["sys-kernel/mykernel-bin-amd-znver3-intel-laptop".to_string()];
        let pkg = resolve("mykernel", &profile, &atoms).unwrap();
        assert!(pkg.atom.ends_with("amd-znver3-intel-laptop"));
        assert_eq!(pkg.combo, "amd-znver3-intel-laptop");
        assert!(pkg.degraded_by > 0);
    }

    #[test]
    fn errors_when_nothing_matches() {
        let profile = Profile {
            cpu: CpuArch::IntelRaptorlake,
            gpu: Gpu::Nvidia,
            platform: Platform::Laptop,
            ram_bytes: 0,
        };
        assert!(resolve("mykernel", &profile, &[]).is_err());
    }
}
