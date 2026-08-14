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

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        Ok(ctx.target.join("etc/fstab").is_file())
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted { id: self.id(), label: self.label().to_string() });
        let parts = ctx.parts.clone().expect("Partition phase must run before Fstab");
        fstab::generate(ctx.runner.as_ref(), &ctx.target, &ctx.layout, &parts).await?;
        let _ = tx.send(Event::PhaseFinished { id: self.id(), duration: std::time::Duration::default() });
        Ok(())
    }

    fn weight(&self) -> u32 {
        1
    }

    fn reversible(&self) -> bool {
        false
    }
}
