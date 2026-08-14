//! Finalize (spec §3, phase 13): sync and unmount. Marking the journal itself complete
//! is the driver's job (CLI/GTK), not this phase's — `Ctx` doesn't hold journal state,
//! only install state, and the driver is what already owns the journal instance.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx};

pub struct FinalizePhase;

#[async_trait::async_trait]
impl Phase for FinalizePhase {
    fn id(&self) -> PhaseId {
        PhaseId::Finalize
    }

    fn label(&self) -> &str {
        "Finishing up"
    }

    async fn is_satisfied(&self, _ctx: &Ctx) -> crate::Result<bool> {
        Ok(false)
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted { id: self.id(), label: self.label().to_string() });

        ctx.runner.run_status("sync", &[]).await?;

        if let Some(parts) = &ctx.parts {
            ctx.runner.run_status("swapoff", &[&parts.swap]).await.ok();
        }
        let target_str = ctx.target_str()?.to_string();
        for rel in ["boot", "var/log", "var", "home", ""] {
            let path = if rel.is_empty() { target_str.clone() } else { format!("{target_str}/{rel}") };
            ctx.runner.run_status("umount", &[&path]).await.ok();
        }

        let _ = tx.send(Event::Complete);
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
