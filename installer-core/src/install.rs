//! Orchestrates the actual install: partition -> stage3 -> store -> kernel -> fstab ->
//! bootloader. Both frontends drive this the same way — call `run` with an
//! `mpsc::UnboundedSender<Progress>`, drain the receiver on their own event loop to
//! render status, and the returned `Result` says whether the whole run succeeded.

use crate::{bootloader, fstab, hardware, kernel, partition, process, stage3, store};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub layout: partition::Layout,
    /// Where the target system gets mounted during install, e.g. `/mnt/gentoo`.
    pub target: PathBuf,
    pub store: store::StoreConfig,
    /// Base package name for kernel atoms in the store, e.g. "mykernel" for
    /// `sys-kernel/mykernel-bin-<combo>`.
    pub kernel_base_name: String,
    /// UI dry-run: walks through the same `Progress` sequence with the same timing
    /// shape, but never touches a disk, the network, or a chroot — for clicking through
    /// the wizard while iterating on the frontend. Hardware detection still runs for
    /// real (it's read-only), so the fake kernel atom reported is at least honest about
    /// what this machine would actually resolve to.
    pub simulate: bool,
}

#[derive(Debug, Clone)]
pub enum Progress {
    Partitioning,
    DownloadingStage3,
    UnpackingStage3,
    ConfiguringStore,
    /// Carries the resolved atom once hardware/kernel matching picks one, so the UI
    /// can show *which* profile got selected (and whether it had to degrade).
    InstallingKernel { atom: String, degraded_by: usize },
    WritingFstab,
    InstallingBootloader,
    Done,
}

pub async fn run(opts: InstallOptions, tx: UnboundedSender<Progress>) -> crate::Result<()> {
    if opts.simulate {
        return run_simulated(opts, tx).await;
    }

    let _ = tx.send(Progress::Partitioning);
    let parts = partition::apply(&opts.layout).await?;
    let target_str = opts
        .target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
    partition::mount_target(&opts.layout, &parts, target_str).await?;

    let _ = tx.send(Progress::DownloadingStage3);
    let source = stage3::resolve_latest().await?;
    let tarball_path = std::env::temp_dir().join("gentoo-installer-stage3.tar.xz");
    stage3::download(&source, &tarball_path).await?;

    let _ = tx.send(Progress::UnpackingStage3);
    stage3::unpack(&tarball_path, &opts.target).await?;
    tokio::fs::remove_file(&tarball_path).await.ok();

    bind_mount_chroot_dirs(&opts.target).await?;

    let _ = tx.send(Progress::ConfiguringStore);
    store::configure(&opts.target, &opts.store).await?;

    let atoms = store::list_binhost_atoms(&opts.store.binhost_url).await?;
    let profile = hardware::Profile::detect()?;
    let kernel_pkg = kernel::resolve(&opts.kernel_base_name, &profile, &atoms)?;
    let _ = tx.send(Progress::InstallingKernel {
        atom: kernel_pkg.atom.clone(),
        degraded_by: kernel_pkg.degraded_by,
    });
    process::run(
        "chroot",
        &[target_str, "emerge", "--usepkgonly", "--getbinpkg", &kernel_pkg.atom],
    )
    .await?;

    let _ = tx.send(Progress::WritingFstab);
    fstab::generate(&opts.target, &opts.layout, &parts).await?;

    let _ = tx.send(Progress::InstallingBootloader);
    bootloader::install(&opts.target, &opts.layout.disk).await?;

    let _ = tx.send(Progress::Done);
    Ok(())
}

/// Fake run for UI iteration: same `Progress` sequence and rough timing shape as the real
/// pipeline, no disk/network/chroot access at all. Hardware detection is real (read-only),
/// so the kernel atom shown at least reflects what this actual machine would resolve to —
/// it just skips the `store::list_binhost_atoms` lookup and reports it as an exact match.
async fn run_simulated(opts: InstallOptions, tx: UnboundedSender<Progress>) -> crate::Result<()> {
    use tokio::time::{sleep, Duration};

    for step in [
        Progress::Partitioning,
        Progress::DownloadingStage3,
        Progress::UnpackingStage3,
        Progress::ConfiguringStore,
    ] {
        let _ = tx.send(step);
        sleep(Duration::from_millis(700)).await;
    }

    let combo = hardware::Profile::detect()
        .map(|p| p.combo())
        .unwrap_or_else(|_| "unknown".to_string());
    let _ = tx.send(Progress::InstallingKernel {
        atom: format!("sys-kernel/{}-bin-{combo}", opts.kernel_base_name),
        degraded_by: 0,
    });
    sleep(Duration::from_millis(700)).await;

    let _ = tx.send(Progress::WritingFstab);
    sleep(Duration::from_millis(500)).await;

    let _ = tx.send(Progress::InstallingBootloader);
    sleep(Duration::from_millis(700)).await;

    let _ = tx.send(Progress::Done);
    Ok(())
}

/// The chroot'd `emerge --sync`/kernel install need a working /proc, /sys, /dev inside
/// the target — otherwise Portage's own sandboxing and device access break immediately.
async fn bind_mount_chroot_dirs(target: &Path) -> crate::Result<()> {
    for dir in ["proc", "sys", "dev"] {
        let target_dir = target.join(dir);
        tokio::fs::create_dir_all(&target_dir).await?;
        let target_str = target_dir
            .to_str()
            .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 path")))?;
        process::run("mount", &["--rbind", &format!("/{dir}"), target_str]).await?;
    }
    Ok(())
}
