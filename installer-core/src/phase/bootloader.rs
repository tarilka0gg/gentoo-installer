//! Bootloader (spec §3, phase 11): Limine only.

use super::{Ctx, Phase, PhaseId};
use crate::bootloader;
use crate::event::{Event, EventTx};

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
        let _ = tx.send(Event::PhaseStarted { id: self.id(), label: self.label().to_string() });
        let parts = ctx.parts.as_ref().ok_or_else(|| crate::Error::Other(anyhow::anyhow!("bootloader phase ran before partitioning")))?;
        bootloader::configure(ctx.runner.as_ref(), &ctx.target, &ctx.layout, parts).await?;
        bootloader::install(ctx.runner.as_ref(), &ctx.target, &ctx.layout.disk).await?;
        let _ = tx.send(Event::PhaseFinished { id: self.id(), duration: std::time::Duration::default() });
        Ok(())
    }

    fn weight(&self) -> u32 {
        2
    }

    fn reversible(&self) -> bool {
        false
    }
}
