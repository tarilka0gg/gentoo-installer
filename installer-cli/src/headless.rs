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
//! | `GENTOO_INSTALLER_WM`, `_PACKAGES`, `_GPU`, `_OPT_LEVEL`, `_PACKAGE_MODE` | same meaning as in the TUI |
//!
//! `--resume` continues the unfinished install this live session remembers on the same disk (the journal
//! is in `/run`), skipping what is already done; without it every phase runs.

use anyhow::{bail, Context, Result};
use installer_core::{
    account::Account,
    command::RealCommandRunner,
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
    let layout = partition::plan(&disk, partition::RootFs::Btrfs, profile.ram_bytes);
    let mut ctx = Ctx::new(
        Arc::new(RealCommandRunner),
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

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
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
                Event::Progress { .. } => {}
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
