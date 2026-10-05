//! Deploy (spec §3, phase 5): unpack the base system and install the matched kernel.
//! ~70% of wall-clock time per spec — the base-system unpack (stage3, currently; see the
//! module doc below) dominates.
//!
//! Deviation from spec: the spec describes this phase as unsquashing a single pre-built
//! root image. This codebase doesn't have a squashfs image pipeline — the base system is
//! an official Gentoo stage3 tarball (`crate::stage3`), fetched and verified here instead
//! of being staged into the live medium ahead of time. Kept as-is rather than blocked on
//! building an image pipeline from scratch; the phase boundary (Deploy = "get a bootable
//! base system onto the target") is the part of the spec that actually matters here, and
//! it holds regardless of which artifact fills it. No `emerge` either way: the kernel is
//! a direct file copy (`kernel::deploy`), never a package install.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx, Level};
use crate::{hardware, kernel, stage3, store};

/// Dropped in the target root after the stage3 unpack succeeded (see `DeployPhase::run`).
pub(super) const STAGE_UNPACKED_MARKER: &str = ".gentoo-installer-stage3-unpacked";

pub struct DeployPhase;

#[async_trait::async_trait]
impl Phase for DeployPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Deploy
    }

    fn label(&self) -> &str {
        "Installing the base system"
    }

    /// Deploy is stage3 + store overlay + kernel, in that order. `etc/portage` only proves the
    /// first step: a run that failed after unpacking the stage3 (say, the kernel download)
    /// would otherwise be skipped on resume and leave a system with no kernel. So all three
    /// results have to be there.
    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        let stage = ctx.target.join("etc/portage").is_dir();
        let overlay = ctx
            .target
            .join("var/db/repos")
            .join(&ctx.store.overlay_name)
            .is_dir();
        let kernel = std::fs::read_dir(ctx.target.join("boot"))
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .any(|e| e.file_name().to_string_lossy().starts_with("vmlinuz-"))
            })
            .unwrap_or(false);
        Ok(stage && overlay && kernel)
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });

        // Written once the stage3 is fully unpacked: a resumed run skips the unpack (minutes) but a run that
        // died halfway through it, which leaves no marker, starts over.
        let unpacked = ctx.target.join(STAGE_UNPACKED_MARKER);
        if unpacked.is_file() {
            let _ = tx.send(log("The stage3 is already unpacked, keeping it."));
        } else {
            let _ = tx.send(log("Resolving current stage3 release..."));
            let source = stage3::resolve(ctx.settings.stage3.as_ref()).await?;
            let tarball_path = std::env::temp_dir().join("gentoo-installer-stage3.tar.xz");

            let _ = tx.send(log("Downloading stage3...".to_string()));
            stage3::download(&source, &tarball_path).await?;

            let _ = tx.send(log("Unpacking stage3...".to_string()));
            stage3::unpack(ctx.runner.as_ref(), &tarball_path, &ctx.target).await?;
            tokio::fs::remove_file(&tarball_path).await.ok();
            tokio::fs::write(&unpacked, "").await?;
        }

        let _ = tx.send(log("Fetching the portage overlay...".to_string()));
        store::configure(ctx.runner.as_ref(), &ctx.target, &ctx.store).await?;

        let _ = tx.send(log("Matching kernel to detected hardware...".to_string()));
        let atoms = store::list_binhost_atoms(&ctx.store.binhost_url).await?;
        let profile = match &ctx.profile {
            Some(p) => p.clone(),
            None => {
                let mut p = hardware::Profile::detect()?;
                if let Some(gpu) = ctx.settings.gpu_override {
                    p.gpu = gpu;
                }
                ctx.profile = Some(p.clone());
                p
            }
        };
        let kernel_pkg = kernel::resolve(&ctx.kernel_base_name, &profile, &atoms)?;
        if kernel_pkg.degraded_by > 0 {
            let _ = tx.send(Event::Log {
                line: format!(
                    "No exact kernel build for {}; using {} instead",
                    profile.combo(),
                    kernel_pkg.combo
                ),
                level: Level::Warn,
            });
        }
        kernel::deploy(
            ctx.runner.as_ref(),
            &ctx.store.binhost_url,
            &kernel_pkg.combo,
            &ctx.target,
        )
        .await?;
        ctx.kernel_pkg = Some(kernel_pkg);

        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        70
    }

    fn reversible(&self) -> bool {
        false
    }
}

fn log(line: impl Into<String>) -> Event {
    Event::Log {
        line: line.into(),
        level: Level::Info,
    }
}
