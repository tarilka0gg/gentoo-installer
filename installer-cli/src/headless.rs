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

use anyhow::{bail, Context, Result};
use installer_core::{
    account::Account,
    command::RealCommandRunner,
    config::StoreEnv,
    event::{Event, Level},
    partition,
    phase::{self, Ctx, Settings},
    stage3::Stage3Source,
    store,
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
    settings.account = Some(Account { username: need("GENTOO_INSTALLER_USERNAME")?, password: need("GENTOO_INSTALLER_PASSWORD")? });
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_HOSTNAME") {
        settings.hostname = v;
    }
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_LOCALES") {
        settings.locales = v.split(',').map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    }
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_TIMEZONE") {
        settings.timezone = v;
    }
    if let Ok(v) = std::env::var("GENTOO_INSTALLER_KEYBOARD") {
        settings.keyboard_layout = v;
    }
    settings.stage3 = std::env::var("GENTOO_INSTALLER_STAGE3_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .map(|u| Stage3Source::custom(u.trim(), std::env::var("GENTOO_INSTALLER_STAGE3_SHA512").ok()));

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
    let result = phase::run_all(&mut ctx, &tx).await;
    drop(tx);
    printer.await.ok();
    result.map_err(|e| anyhow::anyhow!(e))
}
