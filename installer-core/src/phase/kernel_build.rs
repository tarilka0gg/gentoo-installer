//! KernelBuild: optionally builds the kernel for this computer from the kernel matrix (see `crate::kernel_build`) when the
//! store has no kernel for exactly this hardware. Off unless the person asked for it; never fails the install.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx, Level};
use crate::{hardware, kernel_build};

pub struct KernelBuildPhase;

fn profile_of(ctx: &Ctx) -> Option<hardware::Profile> {
    if let Some(p) = &ctx.profile {
        return Some(p.clone());
    }
    let mut p = hardware::Profile::detect().ok()?;
    if let Some(gpu) = ctx.settings.gpu_override {
        p.gpu = gpu;
    }
    Some(p)
}

/// The kernel name the build would install as: the exact combo of this hardware.
fn exact_combo(profile: &hardware::Profile) -> String {
    profile
        .candidates()
        .into_iter()
        .next()
        .unwrap_or_else(|| profile.combo())
}

/// A build is wanted when it was asked for, there is a network (not an offline image) and the kernel installed is not already
/// the one for exactly this hardware (the store's own, or one built by an earlier run).
fn wanted(ctx: &Ctx, profile: &hardware::Profile) -> bool {
    ctx.settings.build_kernel
        && !crate::offline::is_active()
        && !ctx
            .target
            .join("boot")
            .join(format!("vmlinuz-{}", exact_combo(profile)))
            .is_file()
}

#[async_trait::async_trait]
impl Phase for KernelBuildPhase {
    fn id(&self) -> PhaseId {
        PhaseId::KernelBuild
    }

    fn label(&self) -> &str {
        "Building the kernel for this computer"
    }

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        let Some(profile) = profile_of(ctx) else {
            return Ok(true);
        };
        Ok(!wanted(ctx, &profile))
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        if let Some(profile) = profile_of(ctx) {
            let store_kernel = ctx.kernel_pkg.as_ref().map(|k| k.combo.clone());
            match kernel_build::build(
                ctx.runner.as_ref(),
                &ctx.target,
                &profile,
                store_kernel.as_deref(),
                tx,
                self.id(),
            )
            .await
            {
                Ok(combo) => {
                    ctx.kernel_pkg = Some(crate::kernel::KernelPackage {
                        atom: format!("local/{combo}"),
                        combo,
                        degraded_by: 0,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Event::Log {
                        line: format!(
                            "The kernel could not be built here ({e}); the ready-made kernel stays."
                        ),
                        level: Level::Warn,
                    });
                }
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
