//! Packages: the optional software groups the user ticked (`packages::GROUPS`).

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx};
use crate::packages;
use crate::portage_store;
use crate::ustan;
use std::path::Path;

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
        let ustan_done = !ctx.settings.ustan
            || ustan::installed(&ctx.target)
            || !ustan::available(Path::new("/"));
        let store_done = !ctx.settings.portage_store
            || portage_store::installed(&ctx.target)
            || !portage_store::available(Path::new("/"))
            || !ustan::target_has_gtk(&ctx.target);
        Ok(packages::installed(&ctx.target, &ctx.settings.packages) && ustan_done && store_done)
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        packages::install(ctx.runner.as_ref(), &ctx.target, &ctx.settings.packages).await?;
        if ctx.settings.ustan {
            let copied = ustan::install_from_live(Path::new("/"), &ctx.target).await?;
            if !copied.is_empty() {
                let _ = tx.send(Event::Log {
                    line: format!("ustan copied to /usr/local ({})", copied.join(", ")),
                    level: crate::event::Level::Info,
                });
            }
        }
        if ctx.settings.portage_store {
            let copied = portage_store::install_from_live(Path::new("/"), &ctx.target).await?;
            if !copied.is_empty() {
                let _ = tx.send(Event::Log {
                    line: format!("portage-store copied ({})", copied.join(", ")),
                    level: crate::event::Level::Info,
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
        40
    }

    fn reversible(&self) -> bool {
        false
    }
}
