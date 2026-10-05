//! Mount (spec §3, phase 4): mounts the formatted partitions under `ctx.target`.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx};
use crate::partition;

pub struct MountPhase;

#[async_trait::async_trait]
impl Phase for MountPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Mount
    }

    fn label(&self) -> &str {
        "Mounting the target"
    }

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        Ok(ctx.target.join("boot").is_dir()
            && std::fs::read_to_string("/proc/mounts")
                .map(|m| m.contains(&ctx.target.display().to_string()))
                .unwrap_or(false))
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        let parts = ctx
            .parts
            .clone()
            .expect("Partition phase must run before Mount");
        let target_str = ctx.target_str()?.to_string();
        partition::mount_target(ctx.runner.as_ref(), &ctx.layout, &parts, &target_str).await?;
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
