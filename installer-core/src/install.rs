//! Legacy monolithic orchestrator, still what `installer-cli`/`installer-gui` (the
//! current libadwaita-based frontends) drive. The spec-aligned replacement is the
//! `phase/` state machine + `journal` + `Ctx` — this function calls the exact same
//! underlying step functions (`partition`, `stage3`, `store`, `kernel`, `fstab`,
//! `bootloader`) those phases do, just without the journal/resume machinery, so nothing
//! about *what* an install does diverges between the two orchestration layers while
//! `installer-gtk` (spec §2, non-libadwaita) doesn't exist yet.

use crate::account::Account;
use crate::command::{CommandRunner, RealCommandRunner};
use crate::hardware::Gpu;
use crate::wm::WmChoice;
use crate::{account, bootloader, detect, fstab, gpu_driver, hardware, keyboard, kernel, make_conf, partition, stage3, store, timezone, wm};
use std::path::PathBuf;
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
    /// XKB layout code (e.g. "us", "ua") — auto-detected default, or an Advanced-setup
    /// manual choice.
    pub keyboard_layout: String,
    /// IANA zone name (e.g. "Europe/Kyiv") — auto-detected default, or an Advanced-setup
    /// manual choice.
    pub timezone: String,
    pub account: Account,
    /// Compositor to install alongside Noctalia — auto-detected-default shape (same as
    /// `keyboard_layout`/`timezone`): `WmChoice::default()` (niri) unless Advanced setup
    /// picked something else.
    pub wm: WmChoice,
    /// `make.conf`'s `-O2`/`-O3` — `OptLevel::default()` (O2) unless Advanced setup
    /// picked O3.
    pub opt_level: make_conf::OptLevel,
    /// Whether large main-tree packages come down as prebuilt binaries or get compiled
    /// from source — `PackageMode::default()` (Binary) unless Advanced setup picked
    /// Source.
    pub package_mode: make_conf::PackageMode,
    /// Git URL of the wm-configs preset repo `wm::install` clones for the chosen
    /// compositor's config + Noctalia's shared config + tty1-autostart template.
    pub wm_configs_git_url: String,
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
    /// Writes `/etc/portage/make.conf` tuned to the detected hardware — see
    /// `make_conf`'s doc comment.
    WritingMakeConf,
    ConfiguringStore,
    /// Carries the resolved atom once hardware/kernel matching picks one, so the UI
    /// can show *which* profile got selected (and whether it had to degrade).
    InstallingKernel { atom: String, degraded_by: usize },
    /// Only sent when the detected GPU actually needs one (currently: Nvidia). See
    /// `gpu_driver`'s doc comment for why this is the one step that still uses `emerge`.
    InstallingGpuDriver,
    WritingFstab,
    SettingKeyboard,
    SettingTimezone,
    CreatingAccount,
    /// Always sent — the compositor may just be the silent niri default. Same
    /// "always runs, may just be the default" shape as `SettingKeyboard`/`SettingTimezone`.
    InstallingDesktop,
    InstallingBootloader,
    Done,
}

pub async fn run(opts: InstallOptions, tx: UnboundedSender<Progress>) -> crate::Result<()> {
    if opts.simulate {
        return run_simulated(opts, tx).await;
    }

    let runner: &dyn CommandRunner = &RealCommandRunner;

    let _ = tx.send(Progress::Partitioning);
    let parts = partition::create_partitions(runner, &opts.layout).await?;
    partition::format_partitions(runner, &opts.layout, &parts).await?;
    let target_str = opts
        .target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
    partition::mount_target(runner, &opts.layout, &parts, target_str).await?;

    let _ = tx.send(Progress::DownloadingStage3);
    let source = stage3::resolve_latest().await?;
    let tarball_path = std::env::temp_dir().join("gentoo-installer-stage3.tar.xz");
    stage3::download(&source, &tarball_path).await?;

    let _ = tx.send(Progress::UnpackingStage3);
    stage3::unpack(runner, &tarball_path, &opts.target).await?;
    tokio::fs::remove_file(&tarball_path).await.ok();

    let profile = hardware::Profile::detect()?;

    let _ = tx.send(Progress::WritingMakeConf);
    let detected = detect::gather(runner).await;
    let jobs = make_conf::nproc(runner).await;
    make_conf::generate(&opts.target, profile.cpu, &detected, jobs, opts.opt_level, opts.package_mode).await?;

    let _ = tx.send(Progress::ConfiguringStore);
    store::configure(runner, &opts.target, &opts.store).await?;

    let atoms = store::list_binhost_atoms(&opts.store.binhost_url).await?;
    let kernel_pkg = kernel::resolve(&opts.kernel_base_name, &profile, &atoms)?;
    let _ = tx.send(Progress::InstallingKernel {
        atom: kernel_pkg.atom.clone(),
        degraded_by: kernel_pkg.degraded_by,
    });
    // No emerge: the kernel is a direct file copy, not a package install (matches
    // installer-core::phase::deploy's DeployPhase — see its doc comment for why).
    kernel::deploy(runner, &opts.store.binhost_url, &kernel_pkg.combo, &opts.target).await?;

    if profile.gpu == Gpu::Nvidia {
        let _ = tx.send(Progress::InstallingGpuDriver);
        gpu_driver::install(runner, &opts.target).await?;
    }

    let _ = tx.send(Progress::WritingFstab);
    fstab::generate(runner, &opts.target, &opts.layout, &parts).await?;

    let _ = tx.send(Progress::SettingKeyboard);
    keyboard::apply(&opts.target, &opts.keyboard_layout).await?;

    let _ = tx.send(Progress::SettingTimezone);
    timezone::apply(&opts.target, &opts.timezone).await?;

    let _ = tx.send(Progress::CreatingAccount);
    account::create(runner, &opts.target, &opts.account).await?;
    // Root stays locked, so without this the user in `wheel` could never administer the
    // installed system (a stage3 ships neither doas nor sudo). Part of the same step:
    // no new `Progress` variant, so the frontends' exhaustive matches are unaffected.
    account::configure_privilege(&opts.target).await?;
    account::install_doas(runner, &opts.target).await?;

    let _ = tx.send(Progress::InstallingDesktop);
    wm::install(runner, &opts.target, opts.wm, &opts.wm_configs_git_url, &opts.account.username).await?;

    let _ = tx.send(Progress::InstallingBootloader);
    bootloader::configure(runner, &opts.target, &opts.layout, &parts).await?;
    bootloader::install(runner, &opts.target, &opts.layout.disk).await?;

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
        Progress::WritingMakeConf,
        Progress::ConfiguringStore,
    ] {
        let _ = tx.send(step);
        sleep(Duration::from_millis(700)).await;
    }

    let detected_profile = hardware::Profile::detect().ok();
    let combo = detected_profile.as_ref().map(|p| p.combo()).unwrap_or_else(|| "unknown".to_string());
    let _ = tx.send(Progress::InstallingKernel {
        atom: format!("sys-kernel/{}-bin-{combo}", opts.kernel_base_name),
        degraded_by: 0,
    });
    sleep(Duration::from_millis(700)).await;

    if detected_profile.map(|p| p.gpu) == Some(Gpu::Nvidia) {
        let _ = tx.send(Progress::InstallingGpuDriver);
        sleep(Duration::from_millis(700)).await;
    }

    for step in [
        Progress::WritingFstab,
        Progress::SettingKeyboard,
        Progress::SettingTimezone,
        Progress::CreatingAccount,
        Progress::InstallingDesktop,
    ] {
        let _ = tx.send(step);
        sleep(Duration::from_millis(400)).await;
    }

    let _ = tx.send(Progress::InstallingBootloader);
    sleep(Duration::from_millis(700)).await;

    let _ = tx.send(Progress::Done);
    Ok(())
}
