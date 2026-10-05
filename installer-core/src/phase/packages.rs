//! Packages: the optional software groups the user ticked (`packages::GROUPS`).

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx};
use crate::packages;

pub struct PackagesPhase;

#[async_trait::async_trait]
impl Phase for PackagesPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Packages
    }

    fn label(&self) -> &str {
        "Installing the software you chose"
    }

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        Ok(packages::installed(&ctx.target, &ctx.settings.packages))
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        packages::install(ctx.runner.as_ref(), &ctx.target, &ctx.settings.packages).await?;
        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        40
    }

    fn reversible(&self) -> bool {
        false
    }
}
