//! Gpu: the proprietary NVIDIA driver, compiled against the kernel that `Deploy` installed. Every other
//! GPU is served by in-tree drivers and Mesa (selected through `VIDEO_CARDS` in `make.conf`), so for
//! them this phase only says so and finishes.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx, Level};
use crate::gpu_driver;
use crate::hardware::Gpu;

pub struct GpuPhase;

fn needs_driver(ctx: &Ctx) -> bool {
    ctx.profile.as_ref().map(|p| p.gpu) == Some(Gpu::Nvidia)
}

fn nvidia_installed(target: &std::path::Path) -> bool {
    std::fs::read_dir(target.join("var/db/pkg/x11-drivers"))
        .map(|d| {
            d.filter_map(|e| e.ok()).any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("nvidia-drivers-")
            })
        })
        .unwrap_or(false)
}

#[async_trait::async_trait]
impl Phase for GpuPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Gpu
    }

    fn label(&self) -> &str {
        "Installing the graphics driver"
    }

    async fn is_satisfied(&self, ctx: &Ctx) -> crate::Result<bool> {
        Ok(!needs_driver(ctx)
            || nvidia_installed(&ctx.target)
            || gpu_driver::open_driver_selected(&ctx.target).await)
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });
        if needs_driver(ctx) {
            let _ = tx.send(Event::Log {
                line: "Building the NVIDIA driver for the installed kernel...".into(),
                level: Level::Info,
            });
            // The NVIDIA driver is a convenience: when it cannot be built (no prepared kernel tree for this kernel, or the build fails),
            // the install goes on with the open Nouveau driver from Mesa instead of stopping at 28 %.
            if let Err(e) = gpu_driver::install(ctx.runner.as_ref(), &ctx.target).await {
                let _ = tx.send(Event::Log {
                    line: format!("The NVIDIA driver could not be installed: {e}"),
                    level: Level::Warn,
                });
                let has_module = gpu_driver::fall_back_to_open_driver(&ctx.target).await?;
                let line = if has_module {
                    "Using the open Nouveau driver (Mesa) for this NVIDIA card instead: it works, with less speed and none of NVIDIA's own features. Install the NVIDIA driver later from the system once a prepared kernel is available.".to_string()
                } else {
                    "Using the open Nouveau driver (Mesa) for this NVIDIA card, but the installed kernel has no Nouveau module: the screen may stay on the basic framebuffer until a kernel with it is installed.".to_string()
                };
                let _ = tx.send(Event::Log {
                    line,
                    level: Level::Warn,
                });
            }
        } else {
            let gpu = ctx
                .profile
                .as_ref()
                .map(|p| format!("{:?}", p.gpu))
                .unwrap_or_else(|| "unknown".into());
            let _ = tx.send(Event::Log {
                line: format!("No extra driver needed for the {gpu} graphics."),
                level: Level::Info,
            });
        }
        let _ = tx.send(Event::PhaseFinished {
            id: self.id(),
            duration: std::time::Duration::default(),
        });
        Ok(())
    }

    fn weight(&self) -> u32 {
        30
    }

    fn reversible(&self) -> bool {
        false
    }
}
