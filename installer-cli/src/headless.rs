//! `installer-cli --headless`: the whole install with no screens, configured from the
//! environment. For automation and for testing the real phase chain in a VM.
//!
//! It erases a disk, so it refuses to run unless `GENTOO_INSTALLER_CONFIRM_ERASE` names that
//! same disk.
//!
//! | variable | meaning |
//! |---|---|
//! | `GENTOO_INSTALLER_DISK` | e.g. `/dev/vda` (required) |
//! | `GENTOO_INSTALLER_CONFIRM_ERASE` | must equal the disk path (required) |
//! | `GENTOO_INSTALLER_USERNAME` / `_PASSWORD` | the first user (required) |
//! | `GENTOO_STORE_BINHOST_URL`, `GENTOO_STORE_OVERLAY_URL`, … | see `StoreEnv` |
//! | `GENTOO_INSTALLER_HOSTNAME`, `_LOCALES` (comma), `_TIMEZONE`, `_KEYBOARD` | optional |
//! | `GENTOO_INSTALLER_STAGE3_URL`, `_SHA512` | optional custom stage3 (else the bundled one) |
//! | `GENTOO_INSTALLER_USTAN=0` | do not copy ustan from the live image into the installed system |
//! | `GENTOO_INSTALLER_STORE=0` | do not copy portage-store from the live image into the installed system |
//! | `GENTOO_INSTALLER_KERNEL_BUILD=1` | build the kernel for this computer from the kernel matrix when the store has none for exactly this hardware (needs a network, 10-60 min depending on the machine) |
//! | `GENTOO_INSTALLER_SECUREBOOT=1` | sign the installed bootloader with a key made on this machine (UEFI only); enrol `/boot/secureboot/simple-linux-db.cer` afterwards |
//! | `GENTOO_INSTALLER_WM` (`none` = no desktop), `_PACKAGES`, `_GPU`, `_OPT_LEVEL`, `_PACKAGE_MODE` | same meaning as in the TUI |
//!
//! `--resume` continues the unfinished install this live session remembers on the same disk (the journal
//! is in `/run`), skipping what is already done; without it every phase runs.

use anyhow::{bail, Context, Result};
use installer_core::{
    account::Account,
    command::StreamingCommandRunner,
    config::StoreEnv,
    event::{Event, Level},
    journal,
    make_conf::{OptLevel, PackageMode},
    partition,
    phase::{self, Ctx, RunMode, Settings},
    stage3::Stage3Source,
    store,
    wm::WmChoice,
};
use std::sync::Arc;

fn need(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("{key} is not set"))
}

pub async fn run() -> Result<()> {
    let disk = need("GENTOO_INSTALLER_DISK")?;
    let confirm = std::env::var("GENTOO_INSTALLER_CONFIRM_ERASE").unwrap_or_default();
    if confirm != disk {
        bail!("this erases {disk}; set GENTOO_INSTALLER_CONFIRM_ERASE={disk} to go ahead");
    }
    let store_env = StoreEnv::from_env().map_err(|e| anyhow::anyhow!(e))?;

    let mut settings = Settings::default();
    settings.account = Some(Account {
        username: need("GENTOO_INSTALLER_USERNAME")?,
        password: need("GENTOO_INSTALLER_PASSWORD")?,
    });
    // Language and zone follow the keyboard layout unless the environment says otherwise.
    let guessed = installer_core::autodetect::guess_for_layout(
        &std::env::var("GENTOO_INSTALLER_KEYBOARD")
            .unwrap_or_else(|_| installer_core::keyboard::detect_current()),
    );
    settings.locales = guessed.locales;
    if let Some(zone) = guessed.timezone {
        settings.timezone = zone;
    }
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_HOSTNAME") {
        settings.hostname = v;
    }
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_LOCALES") {
        settings.locales = v
            .split(',')
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
    }
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_TIMEZONE") {
        settings.timezone = v;
    }
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_KEYBOARD") {
        settings.keyboard_layout = v;
    }
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_KEYBOARD_OPTIONS") {
        settings.keyboard_options = v;
    }
    settings.desktop = std::env::var("GENTOO_INSTALLER_WM").as_deref() != Ok("none");
    settings.ustan = std::env::var("GENTOO_INSTALLER_USTAN").as_deref() != Ok("0");
    settings.portage_store = std::env::var("GENTOO_INSTALLER_STORE").as_deref() != Ok("0");
    settings.secure_boot = std::env::var("GENTOO_INSTALLER_SECUREBOOT").as_deref() == Ok("1");
    settings.build_kernel = std::env::var("GENTOO_INSTALLER_KERNEL_BUILD").as_deref() == Ok("1");
    settings.wm = match std::env::var("GENTOO_INSTALLER_WM").as_deref() {
        Ok("hyprland") => WmChoice::Hyprland,
        Ok("sway") => WmChoice::Sway,
        Ok("labwc") => WmChoice::Labwc,
        Ok("mangowc") => WmChoice::MangoWc,
        Ok("dwl") => WmChoice::Dwl,
        _ => WmChoice::default(),
    };
    settings.wm_configs_git_url = store_env.wm_configs_git_url.clone();
    settings.packages = match std::env::var("GENTOO_INSTALLER_PACKAGES") {
        Ok(v) => v
            .split(',')
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty())
            .collect(),
        Err(_) => installer_core::packages::default_ids(),
    };
    settings.gpu_override = match std::env::var("GENTOO_INSTALLER_GPU").as_deref() {
        Ok("nvidia") => Some(installer_core::hardware::Gpu::Nvidia),
        Ok("nouveau") => Some(installer_core::hardware::Gpu::Nouveau),
        Ok("amd") => Some(installer_core::hardware::Gpu::Amd),
        Ok("intel") => Some(installer_core::hardware::Gpu::Intel),
        Ok("xe") => Some(installer_core::hardware::Gpu::Xe),
        Ok("none") => Some(installer_core::hardware::Gpu::None),
        _ => None,
    };
    settings.render = render_from_env();
    if matches!(
        std::env::var("GENTOO_INSTALLER_OPT_LEVEL").as_deref(),
        Ok("O3") | Ok("o3")
    ) {
        settings.opt_level = OptLevel::O3;
    }
    if matches!(
        std::env::var("GENTOO_INSTALLER_PACKAGE_MODE").as_deref(),
        Ok("source")
    ) {
        settings.package_mode = PackageMode::Source;
    }
    settings.stage3 = std::env::var("GENTOO_INSTALLER_STAGE3_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .map(|u| {
            Stage3Source::custom(
                u.trim(),
                std::env::var("GENTOO_INSTALLER_STAGE3_SHA512").ok(),
            )
        });

    let profile = installer_core::hardware::Profile::detect().context("hardware detection")?;
    let resuming = std::env::args().any(|a| a == "--resume");
    let layout = match plan_layout(&disk, profile.ram_bytes).await {
        Ok(layout) => layout,
        // The free space has been used by the run that is being resumed; its journal has the layout.
        Err(_) if resuming => partition::plan(&disk, partition::RootFs::Btrfs, profile.ram_bytes),
        Err(e) => return Err(e),
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut ctx = Ctx::new(
        Arc::new(StreamingCommandRunner::new(tx.clone())),
        "/mnt/gentoo".into(),
        layout,
        store::StoreConfig {
            binhost_url: store_env.binhost_url,
            overlay_git_url: store_env.overlay_git_url,
            overlay_name: store_env.overlay_name,
        },
        store_env.kernel_base_name,
    )
    .with_settings(settings);

    let printer = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            match ev {
                Event::PhaseStarted { label, .. } => println!("==> {label}"),
                Event::PhaseFinished { id, .. } => println!("    done: {id:?}"),
                Event::Log { line, level } => {
                    let tag = match level {
                        Level::Info => "   ",
                        Level::Warn => "WARN",
                        Level::Error => "ERR ",
                    };
                    println!("{tag} {line}")
                }
                Event::Progress { .. } | Event::Step { .. } => {}
                Event::Failed { id, error } => println!("FAILED {id:?}: {error}"),
                Event::Complete => println!("INSTALL COMPLETE"),
            }
        }
    });
    let mode = if std::env::args().any(|a| a == "--resume") {
        RunMode::Resume
    } else {
        RunMode::Fresh
    };
    let result = phase::run_phases(&mut ctx, &tx, mode, Some(&journal::default_path())).await;
    drop(tx);
    printer.await.ok();
    result.map_err(|e| anyhow::anyhow!(e))
}

/// `GENTOO_INSTALLER_RENDER=integrated` renders on the integrated GPU; anything else (or unset) takes the
/// most capable adapter, discrete first.
pub fn render_from_env() -> installer_core::gpu::RenderPreference {
    match std::env::var("GENTOO_INSTALLER_RENDER").as_deref() {
        Ok("integrated") => installer_core::gpu::RenderPreference::Integrated,
        _ => installer_core::gpu::RenderPreference::Auto,
    }
}

/// The disk layout from the environment: the whole disk, or with `GENTOO_INSTALLER_PLACEMENT=free` the largest free region of a GPT disk
/// that keeps its partitions (`GENTOO_INSTALLER_FREE_GIB=60` takes only that much of it). The same choice the graphical installer offers.
pub async fn plan_layout(disk: &str, ram_bytes: u64) -> anyhow::Result<partition::Layout> {
    if std::env::var("GENTOO_INSTALLER_PLACEMENT").as_deref() != Ok("free") {
        return Ok(partition::plan(disk, partition::RootFs::Btrfs, ram_bytes));
    }
    let size = std::env::var("GENTOO_INSTALLER_FREE_GIB")
        .ok()
        .and_then(|v| v.trim().parse().ok());
    Ok(partition::plan_next_to_existing(
        &installer_core::command::RealCommandRunner,
        disk,
        partition::RootFs::Btrfs,
        partition::swap_size_gib(ram_bytes),
        size,
    )
    .await?)
}
