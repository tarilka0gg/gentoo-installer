//! PortageConfig: `make.conf` tuned to this machine — `-march=` for the exact CPU, `MAKEOPTS` for its core
//! count, `CPU_FLAGS_X86`, `VIDEO_CARDS` for every GPU present, the binhost settings — through
//! `make_conf::generate`, which keeps what the stage3 shipped and replaces only the keys it owns.
//!
//! It runs right after `Deploy`, before anything is emerged. (An earlier version of this phase wrote a
//! different, minimal `make.conf` over the stage3's and ran after the `doas` emerge; the installer then
//! had two generators that disagreed.) The git history of `/etc/portage` is `PostHooks`' job.

use super::{Ctx, Phase, PhaseId};
use crate::detect;
use crate::event::{Event, EventTx};
use crate::{hardware, make_conf};

pub struct PortageConfigPhase;

#[async_trait::async_trait]
impl Phase for PortageConfigPhase {
    fn id(&self) -> PhaseId {
        PhaseId::PortageConfig
    }

    fn label(&self) -> &str {
        "Tuning make.conf for your hardware"
    }

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        let make_conf_done = std::fs::read_to_string(ctx.target.join("etc/portage/make.conf"))
            .is_ok_and(|t| t.contains(make_conf::GENERATED_MARKER));
        // A binary install also needs the project's binhost; a run that stopped between the two is not done.
        let binhost_done = ctx.settings.package_mode != make_conf::PackageMode::Binary
            || crate::binhost::url_from_env().is_none()
            || crate::binhost::configured(&ctx.target);
        Ok(make_conf_done && binhost_done)
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });

        let profile = match &ctx.profile {
            Some(p) => p.clone(),
            None => {
                let p = hardware::Profile::detect()?;
                ctx.profile = Some(p.clone());
                p
            }
        };
        let mut detected = detect::gather(ctx.runner.as_ref()).await;
        // A GPU the user chose by hand decides the drivers Mesa is built with, too.
        if ctx.settings.gpu_override.is_some() {
            detected.video_cards = Some(detect::video_cards_value(profile.gpu));
        }
        let jobs = make_conf::nproc(ctx.runner.as_ref()).await;
        make_conf::generate(
            &ctx.target,
            profile.cpu,
            &detected,
            jobs,
            ctx.settings.opt_level,
            ctx.settings.package_mode,
        )
        .await?;

        if ctx.settings.package_mode == make_conf::PackageMode::Binary {
            if let Some(url) = crate::binhost::url_from_env() {
                let _ = tx.send(Event::Step {
                    id: self.id(),
                    text: "Connecting the Simple Linux binary package host".into(),
                    fraction: Some(0.5),
                });
                crate::binhost::configure(ctx.runner.as_ref(), &ctx.target, &url).await?;
            }
        }

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
