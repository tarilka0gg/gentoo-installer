//! Builds the kernel for this computer on the new system, from the [kernel matrix](https://github.com/tarilka0gg/kernel-matrix):
//! the same fragments (`common/`, `cpu/`, `gpu/`, `platform/`, `ec/all`, `modem/all`) on top of the same seed the ready-made
//! kernels of the store are made from. Only used when the person asked for it (Advanced setup, Optimization) and the store has
//! no kernel for exactly this hardware; otherwise the store's kernel (or its closest, the generic one) stays. It needs a network,
//! about 10 GB of free space and 15 to 60 minutes. Any failure leaves the store's kernel in place: this never makes an install fail.

use crate::command::CommandRunner;
use crate::event::{Event, EventTx, Level};
use crate::hardware::{Gpu, Profile};
use crate::phase::PhaseId;
use std::path::Path;

/// The matrix repository (`GENTOO_KERNEL_MATRIX_URL` replaces it, for a mirror or a test).
pub const MATRIX_GIT_URL: &str = "https://github.com/tarilka0gg/kernel-matrix.git";
/// The kernel the matrix was made for: its seed is a 7.1.x config.
pub const KERNEL_VERSION: &str = "7.1.8";
/// Written into the new system when its kernel was built here: the update tool then leaves the kernel alone.
pub const MARKER: &str = "etc/simple-linux/local-kernel";

pub fn matrix_url() -> String {
    std::env::var("GENTOO_KERNEL_MATRIX_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| MATRIX_GIT_URL.to_string())
}

/// Where kernel.org keeps `linux-<version>.tar.xz` (`v7.x` for 7.1.8).
pub fn tarball_url(version: &str) -> String {
    let major = version.split('.').next().unwrap_or("7");
    format!("https://cdn.kernel.org/pub/linux/kernel/v{major}.x/linux-{version}.tar.xz")
}

pub fn checksums_url(version: &str) -> String {
    let major = version.split('.').next().unwrap_or("7");
    format!("https://cdn.kernel.org/pub/linux/kernel/v{major}.x/sha256sums.asc")
}

/// The sha256 kernel.org lists for `file` in its `sha256sums.asc` (`<hex>  <file>` lines).
pub fn checksum_of(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let (hash, name) = (parts.next()?, parts.next()?);
        (name == file && hash.len() == 64).then(|| hash.to_string())
    })
}

/// `make -j` for this machine: every thread, but at most one per 2 GiB of RAM (the link steps of a kernel are hungry).
pub fn jobs_for(threads: usize, ram_bytes: u64) -> usize {
    let by_ram = (ram_bytes / (2 << 30)).max(1) as usize;
    threads.clamp(1, by_ram.max(1))
}

/// The shell script run inside the new system: the matrix's seed and fragments merged the way `build-config.sh` does, then
/// the build and `modules_install`. `compiler` is the `make` arguments for the compiler (`CC=gcc` or `CC=clang LD=ld.lld`).
pub fn build_script(src: &str, fragments: &[String], jobs: usize, compiler: &str) -> String {
    let frags: String = fragments
        .iter()
        .map(|f| format!(" /var/tmp/kernel-matrix/{f}.config"))
        .collect();
    format!(
        "set -e\n\
         export LC_ALL=C\n\
         cd {src}\n\
         cp /var/tmp/kernel-matrix/seed/default.config .config\n\
         scripts/kconfig/merge_config.sh -m .config /var/tmp/kernel-matrix/common/*.config{frags} > /var/tmp/kernel-merge.log 2>&1\n\
         make {compiler} olddefconfig\n\
         make {compiler} -j{jobs}\n\
         make {compiler} modules_install\n"
    )
}

/// The parts of a built tree that an out-of-tree module (the NVIDIA driver) needs: the same list the matrix puts in a
/// store kernel's `-devel.tar.xz`.
pub const DEVEL_PARTS: &[&str] = &[
    ".config",
    "Makefile",
    "Module.symvers",
    "Kconfig",
    "scripts",
    "include",
    "arch/x86/include",
    "arch/x86/Makefile",
    "arch/x86/Kbuild",
    "tools/objtool/objtool",
    "certs",
];

fn log(tx: &EventTx, level: Level, line: impl Into<String>) {
    let _ = tx.send(Event::Log {
        line: line.into(),
        level,
    });
}

fn step(tx: &EventTx, id: PhaseId, text: &str, fraction: Option<f64>) {
    let _ = tx.send(Event::Step {
        id,
        text: text.to_string(),
        fraction,
    });
}

fn s(p: &Path) -> crate::Result<&str> {
    p.to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 path {}", p.display())))
}

/// What is in `<target>/lib/modules` (the module trees of the store's kernel, before the build adds its own).
fn module_trees(target: &Path) -> Vec<String> {
    std::fs::read_dir(target.join("lib/modules"))
        .map(|d| {
            d.filter_map(|e| e.ok()?.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Builds and installs the kernel for `profile` into `target` (a mounted new system with its base installed and its Portage
/// configured). Returns the combo name the kernel is installed as (`/boot/vmlinuz-<combo>`).
pub async fn build(
    runner: &dyn CommandRunner,
    target: &Path,
    profile: &Profile,
    store_kernel: Option<&str>,
    tx: &EventTx,
    id: PhaseId,
) -> crate::Result<String> {
    let combo = profile
        .candidates()
        .into_iter()
        .next()
        .unwrap_or_else(|| profile.combo());
    let target_str = s(target)?;
    let version = KERNEL_VERSION;
    let src_rel = format!("usr/src/linux-{version}");
    let src_in_chroot = format!("/{src_rel}");

    // 1. The sources and the matrix, fetched on the live system (it has git and a network).
    step(
        tx,
        id,
        "Fetching the kernel sources and the kernel matrix",
        Some(0.02),
    );
    let tarball = target.join(format!("var/tmp/linux-{version}.tar.xz"));
    tokio::fs::create_dir_all(target.join("var/tmp")).await?;
    let file = format!("linux-{version}.tar.xz");
    crate::http::download_to_file(&tarball_url(version), &tarball).await?;
    let sums = std::env::temp_dir().join("simple-linux-kernel-sums");
    crate::http::download_to_file(&checksums_url(version), &sums).await?;
    let expected = checksum_of(&std::fs::read_to_string(&sums).unwrap_or_default(), &file)
        .ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("kernel.org lists no checksum for {file}"))
        })?;
    tokio::fs::remove_file(&sums).await.ok();
    let actual = {
        use sha2::{Digest, Sha256};
        let bytes = tokio::fs::read(&tarball).await?;
        hex::encode(Sha256::digest(&bytes))
    };
    if actual != expected {
        tokio::fs::remove_file(&tarball).await.ok();
        return Err(crate::Error::Other(anyhow::anyhow!(
            "the kernel tarball does not match kernel.org's checksum"
        )));
    }
    log(tx, Level::Info, "Unpacking the kernel sources...");
    runner
        .run_status(
            "tar",
            &["-xJf", s(&tarball)?, "-C", s(&target.join("usr/src"))?],
        )
        .await?;
    tokio::fs::remove_file(&tarball).await.ok();
    let matrix_dir = target.join("var/tmp/kernel-matrix");
    tokio::fs::remove_dir_all(&matrix_dir).await.ok();
    runner
        .run_status(
            "git",
            &["clone", "--depth", "1", &matrix_url(), s(&matrix_dir)?],
        )
        .await?;

    // 2. The tools the kernel build needs on top of a stage3 (binary packages), inside the new system.
    step(tx, id, "Installing the build tools", Some(0.06));
    crate::chroot_emerge::bind_mount_chroot_dirs(runner, target).await?;
    let result = async {
        crate::chroot_emerge::ensure_network_resolves(target).await?;
        crate::chroot_emerge::ensure_portage_tree(runner, target, target_str).await?;
        runner
            .run_status(
                "chroot",
                &[
                    target_str,
                    "emerge",
                    "-1",
                    "-q",
                    "--noreplace",
                    "--getbinpkg",
                    "sys-devel/bc",
                    "dev-libs/elfutils",
                    "sys-libs/binutils-libs",
                ],
            )
            .await?;

        // 3. Configure and build: gcc first; clang when gcc's build fails (a known gcc/objtool problem on some configs).
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        let ram = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|m| {
                m.lines()
                    .find(|l| l.starts_with("MemTotal:"))
                    .and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
            })
            .unwrap_or(4 << 20)
            * 1024;
        let jobs = jobs_for(threads, ram);
        step(
            tx,
            id,
            &format!("Building the kernel for this computer ({jobs} jobs, this takes a while)"),
            Some(0.10),
        );
        log(
            tx,
            Level::Info,
            format!("Building {combo} from the kernel matrix with {jobs} jobs..."),
        );
        let fragments = profile.fragments();
        let gcc = build_script(&src_in_chroot, &fragments, jobs, "CC=gcc");
        if let Err(first) = runner
            .run_status("chroot", &[target_str, "/bin/bash", "-c", &gcc])
            .await
        {
            log(
                tx,
                Level::Warn,
                format!("The build with gcc failed ({first}); trying clang."),
            );
            step(tx, id, "Building the kernel with clang", Some(0.15));
            runner
                .run_status(
                    "chroot",
                    &[
                        target_str,
                        "emerge",
                        "-1",
                        "-q",
                        "--noreplace",
                        "--getbinpkg",
                        "llvm-core/clang",
                        "llvm-core/lld",
                    ],
                )
                .await?;
            let clang = build_script(&src_in_chroot, &fragments, jobs, "CC=clang LD=ld.lld");
            runner
                .run_status("chroot", &[target_str, "/bin/bash", "-c", &clang])
                .await?;
        }
        Ok::<(), crate::Error>(())
    }
    .await;
    crate::chroot_emerge::unmount_chroot_dirs(runner, target).await;
    result?;

    // 4. Install: the built kernel replaces the store's.
    step(tx, id, "Installing the new kernel", Some(0.95));
    let src = target.join(&src_rel);
    let bzimage = src.join("arch/x86/boot/bzImage");
    if !bzimage.is_file() {
        return Err(crate::Error::Other(anyhow::anyhow!(
            "the build left no bzImage"
        )));
    }
    let boot = target.join("boot");
    tokio::fs::copy(&bzimage, boot.join(format!("vmlinuz-{combo}"))).await?;
    if let Some(old) = store_kernel.filter(|o| *o != combo) {
        tokio::fs::remove_file(boot.join(format!("vmlinuz-{old}")))
            .await
            .ok();
    }
    // The module trees that were there before (the store kernel's) are dropped, the new one stays.
    let built_release = std::fs::read_to_string(src.join("include/config/kernel.release"))
        .map(|r| r.trim().to_string())
        .unwrap_or_default();
    for tree in module_trees(target) {
        if tree != built_release && !built_release.is_empty() {
            tokio::fs::remove_dir_all(target.join("lib/modules").join(&tree))
                .await
                .ok();
        }
    }
    // The proprietary NVIDIA driver is built against a prepared tree: the same parts a store kernel's -devel tarball has.
    if profile.gpu == Gpu::Nvidia {
        let dst = target.join(format!("usr/src/linux-{combo}"));
        tokio::fs::create_dir_all(&dst).await?;
        let mut args: Vec<String> = vec![
            "-C".into(),
            s(&src)?.into(),
            "-cf".into(),
            s(&dst.join("devel.tar"))?.into(),
        ];
        args.extend(
            DEVEL_PARTS
                .iter()
                .filter(|p| src.join(p).exists())
                .map(|p| p.to_string()),
        );
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        runner.run_status("tar", &args).await?;
        runner
            .run_status("tar", &["-xpf", s(&dst.join("devel.tar"))?, "-C", s(&dst)?])
            .await?;
        tokio::fs::remove_file(dst.join("devel.tar")).await.ok();
        let link = target.join("usr/src/linux");
        tokio::fs::remove_file(&link).await.ok();
        tokio::fs::symlink(format!("linux-{combo}"), &link).await?;
    }
    tokio::fs::remove_dir_all(&src).await.ok();
    tokio::fs::remove_dir_all(&matrix_dir).await.ok();
    tokio::fs::remove_file(target.join("var/tmp/kernel-merge.log"))
        .await
        .ok();
    tokio::fs::create_dir_all(target.join("etc/simple-linux")).await?;
    tokio::fs::write(
        target.join(MARKER),
        format!("{combo}\nbuilt on this computer from the kernel matrix, linux {version}\n"),
    )
    .await?;
    log(
        tx,
        Level::Info,
        format!("The kernel for this computer is installed as vmlinuz-{combo}."),
    );
    Ok(combo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{CpuArch, Platform};

    #[test]
    fn kernel_org_urls_follow_the_major_version() {
        assert_eq!(
            tarball_url("7.1.8"),
            "https://cdn.kernel.org/pub/linux/kernel/v7.x/linux-7.1.8.tar.xz"
        );
        assert!(checksums_url("7.1.8").ends_with("/v7.x/sha256sums.asc"));
    }

    #[test]
    fn the_checksum_of_a_file_is_read_from_the_sums_list() {
        let h = "a".repeat(64);
        let sums = format!(
            "-----BEGIN PGP SIGNED MESSAGE-----\n{h}  linux-7.1.8.tar.xz\n{}  linux-7.1.7.tar.xz\n",
            "b".repeat(64)
        );
        assert_eq!(checksum_of(&sums, "linux-7.1.8.tar.xz"), Some(h));
        assert_eq!(checksum_of(&sums, "linux-9.9.tar.xz"), None);
    }

    #[test]
    fn jobs_follow_threads_but_not_beyond_two_gib_of_ram_each() {
        assert_eq!(jobs_for(16, 64 << 30), 16);
        assert_eq!(jobs_for(16, 8 << 30), 4);
        assert_eq!(jobs_for(8, 1 << 30), 1);
        assert_eq!(jobs_for(0, 0), 1);
    }

    #[test]
    fn the_script_merges_the_seed_and_the_fragments_in_the_matrix_order() {
        let p = Profile {
            cpu: CpuArch::IntelRaptorlake,
            gpu: Gpu::Nvidia,
            platform: Platform::Laptop,
            ram_bytes: 0,
        };
        let script = build_script("/usr/src/linux-7.1.8", &p.fragments(), 8, "CC=gcc");
        assert!(script.contains("cp /var/tmp/kernel-matrix/seed/default.config .config"));
        let merge = script
            .lines()
            .find(|l| l.contains("merge_config.sh"))
            .unwrap();
        let order = [
            "common/*.config",
            "cpu/intel-raptorlake.config",
            "gpu/nvidia.config",
            "platform/laptop.config",
            "ec/all.config",
            "modem/all.config",
        ];
        let positions: Vec<usize> = order.iter().map(|o| merge.find(o).expect(o)).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{merge}");
        assert!(script.contains("make CC=gcc -j8\n") && script.contains("modules_install"));
    }
}
