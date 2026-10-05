//! Fstab (spec §3, phase 6).

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx};
use crate::fstab;

pub struct FstabPhase;

#[async_trait::async_trait]
impl Phase for FstabPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Fstab
    }

    fn label(&self) -> &str {
        "Writing the filesystem table"
    }

    /// A stage3 ships its own `/etc/fstab` (comments only), so "the file exists" says nothing:
    /// the first real install skipped this phase on that basis and booted with an empty fstab
    /// (no `/home`, no `/var`, no `/boot`, no swap). Only the installer's own output counts.
    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        Ok(std::fs::read_to_string(ctx.target.join("etc/fstab"))
            .is_ok_and(|t| t.contains(fstab::GENERATED_MARKER)))
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        let parts = ctx.parts.clone().ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!(
                "the fstab needs the partition layout; run the Partition phase first"
            ))
        })?;
        fstab::generate(ctx.runner.as_ref(), &ctx.target, &ctx.layout, &parts).await?;
        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        1
    }

    fn reversible(&self) -> bool {
        false
    }
}
