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
        // A fresh install runs every phase without asking `is_satisfied` (only a resume does), so this one has to look at
        // `wanted` itself: without it every install with a network would download the kernel sources and compile a kernel the
        // person never asked for.
        let Some(profile) = profile_of(ctx).filter(|p| wanted(ctx, p)) else {
            let _ = tx.send(Event::PhaseFinished {
                id: self.id(),
                duration: std::time::Duration::default(),
            });
            return Ok(());
        };
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;
    use std::sync::Arc;

    fn ctx(build_kernel: bool, runner: Arc<FakeCommandRunner>) -> Ctx {
        let layout = crate::partition::plan("/dev/sda", crate::partition::RootFs::Btrfs, 16 << 30);
        let store = crate::store::StoreConfig {
            binhost_url: "https://example.invalid".into(),
            overlay_git_url: String::new(),
            overlay_name: "test".into(),
        };
        let mut c = Ctx::new(runner, std::env::temp_dir().join("gi-kbuild-test"), layout, store, "k".into());
        c.profile = Some(hardware::Profile {
            cpu: hardware::CpuArch::IntelRaptorlake,
            gpu: hardware::Gpu::Intel,
            platform: hardware::Platform::Laptop,
            ram_bytes: 16 << 30,
        });
        c.settings.build_kernel = build_kernel;
        c
    }

    #[tokio::test]
    async fn a_fresh_install_that_did_not_ask_for_a_kernel_build_does_nothing_here() {
        let runner = Arc::new(FakeCommandRunner::new());
        let mut c = ctx(false, runner.clone());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        KernelBuildPhase.run(&mut c, &tx).await.unwrap();
        drop(tx);
        assert!(runner.calls().is_empty(), "no command ran: {:?}", runner.calls());
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        assert!(
            matches!(events.as_slice(), [Event::PhaseFinished { .. }]),
            "no \"Building the kernel\" label for a build that is not happening: {events:?}"
        );
        assert!(c.kernel_pkg.is_none());
        assert!(KernelBuildPhase.is_satisfied(&c).await.unwrap(), "and a resume skips it too");
    }

    #[tokio::test]
    async fn when_it_was_asked_for_and_there_is_a_network_a_build_is_wanted() {
        // (only the decision: `run` would fetch the kernel sources from kernel.org)
        let c = ctx(true, Arc::new(FakeCommandRunner::new()));
        let profile = profile_of(&c).unwrap();
        assert!(wanted(&c, &profile));
        assert!(!KernelBuildPhase.is_satisfied(&c).await.unwrap());
        // not when the kernel for exactly this hardware is already installed
        let boot = c.target.join("boot");
        std::fs::create_dir_all(&boot).unwrap();
        std::fs::write(boot.join(format!("vmlinuz-{}", exact_combo(&profile))), "k").unwrap();
        assert!(!wanted(&c, &profile));
        std::fs::remove_dir_all(&c.target).ok();
    }
}
