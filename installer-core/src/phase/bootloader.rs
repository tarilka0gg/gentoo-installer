//! Bootloader (spec §3, phase 11): Limine only.

use super::{Ctx, Phase, PhaseId};
use crate::bootloader;
use crate::event::{Event, EventTx, Level};
use crate::sltools;
use std::path::Path;

pub struct BootloaderPhase;

#[async_trait::async_trait]
impl Phase for BootloaderPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Bootloader
    }

    fn label(&self) -> &str {
        "Installing the bootloader"
    }

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        let installed = ctx.target.join("boot/limine/limine-bios.sys").is_file()
            || ctx.target.join("boot/EFI/BOOT/BOOTX64.EFI").is_file();
        Ok(installed && ctx.target.join("boot/limine.conf").is_file())
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        let parts = ctx.parts.as_ref().ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!("bootloader phase ran before partitioning"))
        })?;
        // The proprietary NVIDIA driver needs kernel modesetting for a Wayland session to get an output.
        let extra = if ctx.profile.as_ref().map(|p| p.gpu) == Some(crate::hardware::Gpu::Nvidia) {
            "nvidia-drm.modeset=1"
        } else {
            ""
        };
        bootloader::configure(ctx.runner.as_ref(), &ctx.target, &ctx.layout, parts, extra).await?;
        bootloader::install(ctx.runner.as_ref(), &ctx.target, &ctx.layout.disk).await?;
        if let Some(pkg) = &ctx.kernel_pkg {
            sltools::install_update_tool(&ctx.target, &ctx.store.binhost_url, &pkg.combo).await?;
        }
        if ctx.settings.secure_boot {
            if Path::new("/sys/firmware/efi").is_dir() {
                sltools::setup_secure_boot(ctx.runner.as_ref(), &ctx.target).await?;
                let _ = tx.send(Event::Log {
                    line: "Secure Boot: the bootloader is signed with this machine's own key. Enrol /boot/secureboot/simple-linux-db.cer in the firmware, then turn Secure Boot on.".into(),
                    level: Level::Info,
                });
            } else {
                let _ = tx.send(Event::Log {
                    line: "Secure Boot was asked for, but this is a BIOS boot: skipped.".into(),
                    level: Level::Warn,
                });
            }
        }
        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        2
    }

    fn reversible(&self) -> bool {
        false
    }
}
