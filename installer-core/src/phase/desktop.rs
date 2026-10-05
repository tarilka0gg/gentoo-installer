//! Desktop: the chosen compositor, Noctalia and the user's session files (`wm::install`), plus the
//! OpenRC services and runtime directory the session needs.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx};
use crate::wm;

pub struct DesktopPhase;

#[async_trait::async_trait]
impl Phase for DesktopPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Desktop
    }

    fn label(&self) -> &str {
        "Installing the desktop"
    }

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        if !ctx.settings.desktop {
            return Ok(true);
        }
        let Some(account) = &ctx.settings.account else {
            return Ok(false);
        };
        Ok(wm::installed(&ctx.target, ctx.settings.wm)
            && ctx
                .target
                .join("home")
                .join(&account.username)
                .join(".bash_profile")
                .is_file())
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        if !ctx.settings.desktop {
            let _ = tx.send(Event::PhaseFinished {
                id: self.id(),
                duration: std::time::Duration::default(),
            });
            return Ok(());
        }
        let account = ctx.settings.account.clone().ok_or_else(|| {
            crate::Error::Other(anyhow::anyhow!(
                "the desktop is set up for a user; none was given"
            ))
        })?;
        let gpus = match ctx.runner.run("lspci", &["-nn", "-D"]).await {
            Ok(text) => crate::gpu::parse_lspci(&text),
            Err(_) => Vec::new(),
        };
        let nouveau = ctx.settings.gpu_override == Some(crate::hardware::Gpu::Nouveau)
            || ctx
                .profile
                .as_ref()
                .is_some_and(|p| p.gpu == crate::hardware::Gpu::Nouveau);
        let render = crate::gpu::RenderPlan::new(&gpus, ctx.settings.render, nouveau);
        wm::install(
            ctx.runner.as_ref(),
            &ctx.target,
            ctx.settings.wm,
            &ctx.settings.wm_configs_git_url,
            &account.username,
            &render,
        )
        .await?;
        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        100
    }

    fn reversible(&self) -> bool {
        false
    }
}
