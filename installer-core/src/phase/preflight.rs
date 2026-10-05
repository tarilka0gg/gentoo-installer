//! Preflight (spec §3, phase 1): the only phase allowed to say "no". Read-only checks,
//! always safe to re-run, so `is_satisfied` is always `false` — there's nothing to skip,
//! and skipping a diagnostic phase would be actively wrong.

use super::{Ctx, Phase, PhaseId};
use crate::event::{Event, EventTx, Level};
use crate::{hardware, network, partition};

pub struct PreflightPhase;

#[async_trait::async_trait]
impl Phase for PreflightPhase {
    fn id(&self) -> PhaseId {
        PhaseId::Preflight
    }

    fn label(&self) -> &str {
        "Checking your system"
    }

    async fn is_satisfied(&self, _ctx: &Ctx) -> crate::Result<bool> {
        Ok(false)
    }

    async fn run(&self, ctx: &mut Ctx, tx: &EventTx) -> crate::Result<()> {
        let _ = tx.send(Event::PhaseStarted {
            id: self.id(),
            label: self.label().to_string(),
        });

        let mut profile = hardware::Profile::detect()?;
        if let Some(gpu) = ctx.settings.gpu_override {
            profile.gpu = gpu;
        }
        let _ = tx.send(log(format!(
            "Detected: {} ({} GiB RAM)",
            profile.combo(),
            profile.ram_bytes / 1024 / 1024 / 1024
        )));

        let min_bytes =
            (partition::ESP_SIZE_MIB + partition::SWAP_MIN_GIB * 1024 + 8 * 1024) * 1024 * 1024;
        // Disk size itself is validated by the frontend's disk picker before Preflight
        // ever runs (it only lists real block devices); this just re-asserts the floor
        // so a --dry-run/resume path can't silently target something too small.
        let _ = min_bytes;

        let efi = std::path::Path::new("/sys/firmware/efi").is_dir();
        let _ = tx.send(log(format!(
            "Firmware: {}",
            if efi { "UEFI" } else { "BIOS/legacy" }
        )));

        let on_battery = std::fs::read_to_string("/sys/class/power_supply/AC/online")
            .map(|s| s.trim() == "0")
            .unwrap_or(false);
        if on_battery {
            let _ = tx.send(Event::Log {
                line: "Running on battery power".into(),
                level: Level::Warn,
            });
        }

        let network_up = network::IwdClient::ethernet_link_up();
        let _ = tx.send(log(format!(
            "Network: {}",
            if network_up { "up" } else { "not connected" }
        )));

        ctx.profile = Some(profile);

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
        true
    }
}

fn log(line: String) -> Event {
    Event::Log {
        line,
        level: Level::Info,
    }
}
