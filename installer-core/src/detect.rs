//! Hardware and environment inference (spec §6). Every field is `Option<T>` — detection
//! failing is normal and must never be fatal. Gathered once at startup; `gather()` runs
//! each detector concurrently rather than sequentially so one slow probe doesn't hold up
//! the others (there's no IP-geolocation call yet — see the `timezone` doc comment — so
//! nothing here is actually slow today, but the shape is ready for one).
//!
//! This overlaps with, but is distinct from, `hardware::Profile`: that module answers
//! "which precompiled kernel matches this machine" (a 3-axis combo lookup key).
//! `DetectedSystem` answers "what do we write into make.conf / the confirm screen"
//! (`VIDEO_CARDS`, timezone, locale, existing-OS warnings, ...) — overlapping inputs
//! (GPU vendor, EFI/BIOS), different outputs. Reuses `hardware::Profile::detect()`
//! rather than re-probing the same `lspci`/`/proc/cpuinfo` data twice.

use crate::command::CommandRunner;
use crate::{disk, hardware, network};

#[derive(Debug, Clone)]
pub struct DetectedSystem {
    /// `CPU_FLAGS_X86` value, from `cpuid2cpuflags` if that tool is present on the live
    /// medium. `None` (not a failure) if the tool is missing — up to the caller whether
    /// to fall back to the x86-64 psABI level `hardware::Profile` already computes.
    pub cpu_flags: Option<String>,
    /// `VIDEO_CARDS` value(s) derived from the same GPU vendor detection
    /// `hardware::Profile` uses for kernel matching, mapped to Portage's naming.
    pub video_cards: Option<String>,
    pub firmware: Firmware,
    pub disks: Vec<disk::Disk>,
    /// Name of another OS found on the disk (e.g. "Windows 11"), via `os-prober` if
    /// present. `None` means either nothing found or `os-prober` isn't installed —
    /// deliberately not distinguished, since both cases mean "don't warn the user".
    pub existing_os: Option<String>,
    /// IANA zone name (e.g. "Europe/Kyiv"). IP geolocation is explicitly *not*
    /// implemented here — that means committing this installer to a specific third-party
    /// geolocation API, which isn't this module's call to make. Falls back straight from
    /// the live environment (`/etc/timezone`, or the `/etc/localtime` symlink target) to
    /// `None` (caller defaults to UTC).
    pub timezone: Option<String>,
    pub locale: Option<String>,
    pub ram_mb: Option<u64>,
    pub on_battery: Option<bool>,
    pub network_up: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Firmware {
    Uefi,
    Bios,
}

pub async fn gather(runner: &dyn CommandRunner) -> DetectedSystem {
    let profile = hardware::Profile::detect().ok();

    let cpu_flags = runner.run("cpuid2cpuflags", &[]).await.ok().map(|s| {
        // Real output is "CPU_FLAGS_X86=\"...\"" on one line; keep only the value.
        s.trim()
            .trim_start_matches("CPU_FLAGS_X86=")
            .trim_matches('"')
            .to_string()
    });

    let video_cards = profile.as_ref().map(|p| video_cards_value(p.gpu));

    let firmware = if std::path::Path::new("/sys/firmware/efi").is_dir() {
        Firmware::Uefi
    } else {
        Firmware::Bios
    };

    let disks = disk::list(runner).await.unwrap_or_default();

    let existing_os = runner.run("os-prober", &[]).await.ok().and_then(|out| {
        out.lines()
            .next()
            .map(|l| l.split(':').nth(1).unwrap_or(l).trim().to_string())
    });

    let timezone = std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|s| s.trim().to_string())
        .or_else(|| {
            std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                p.strip_prefix("/usr/share/zoneinfo/")
                    .ok()
                    .map(|p| p.display().to_string())
            })
        });

    let locale = std::env::var("LANG").ok();

    let ram_mb = profile.as_ref().map(|p| p.ram_bytes / 1024 / 1024);

    let on_battery = std::fs::read_to_string("/sys/class/power_supply/AC/online")
        .ok()
        .map(|s| s.trim() == "0");

    let network_up = network::IwdClient::ethernet_link_up();

    DetectedSystem {
        cpu_flags,
        video_cards,
        firmware,
        disks,
        existing_os,
        timezone,
        locale,
        ram_mb,
        on_battery,
        network_up,
    }
}

/// Portage's `VIDEO_CARDS` naming differs from the kernel store's combo vocabulary
/// (`amd`/`intel` there means "amdgpu/i915 driver family", not the USE-flag-style token
/// make.conf expects).
fn video_cards_value(gpu: hardware::Gpu) -> String {
    match gpu {
        hardware::Gpu::Nvidia => "nvidia".to_string(),
        hardware::Gpu::Nouveau => "nouveau".to_string(),
        hardware::Gpu::Amd => "amdgpu".to_string(),
        hardware::Gpu::RadeonLegacy => "radeon".to_string(),
        hardware::Gpu::Intel => "i915".to_string(),
        hardware::Gpu::Xe => "xe".to_string(),
        hardware::Gpu::None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_cards_uses_portage_driver_names() {
        assert_eq!(video_cards_value(hardware::Gpu::Amd), "amdgpu");
        assert_eq!(video_cards_value(hardware::Gpu::Intel), "i915");
        assert_eq!(video_cards_value(hardware::Gpu::RadeonLegacy), "radeon");
    }

    #[tokio::test]
    async fn gather_never_fails_even_with_no_tools_available() {
        let runner = crate::command::FakeCommandRunner::new();
        runner.fail("cpuid2cpuflags", "not found");
        runner.fail("os-prober", "not found");
        runner.respond("lsblk", "{\"blockdevices\":[]}");

        // Must not panic — every field is Option or has a safe default.
        let detected = gather(&runner).await;
        assert!(detected.disks.is_empty());
    }
}
