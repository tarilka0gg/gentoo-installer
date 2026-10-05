//! Partition + Format (spec §3, phases 2-3): the point of no return. Not reversible —
//! `create_partitions` wipes the disk's partition table unconditionally.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx};
use crate::partition;

pub struct PartitionPhase;

#[async_trait::async_trait]
impl Phase for PartitionPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Partition
    }

    fn label(&self) -> &str {
        "Partitioning the disk"
    }

    /// Never satisfied by inspection alone — re-partitioning is destructive, so "already
    /// done" has to come from the journal (a prior `Done` record), not a live probe that
    /// might guess wrong and skip a step that didn't actually finish.
    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        Ok(ctx.parts.is_some())
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        let parts = partition::create_partitions(ctx.runner.as_ref(), &ctx.layout).await?;
        ctx.parts = Some(parts);
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

    fn trusts_journal(&self) -> bool {
        true
    }
}

pub struct FormatPhase;

#[async_trait::async_trait]
impl Phase for FormatPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Format
    }

    fn label(&self) -> &str {
        "Formatting partitions"
    }

    async fn is_satisfied(&self, _ctx: &Ctx) -> crate::Result<bool> {
        Ok(false)
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        let parts = ctx
            .parts
            .clone()
            .expect("Partition phase must run before Format");
        partition::format_partitions(ctx.runner.as_ref(), &ctx.layout, &parts).await?;
        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        3
    }

    fn reversible(&self) -> bool {
        false
    }

    fn trusts_journal(&self) -> bool {
        true
    }
}
