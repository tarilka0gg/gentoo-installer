//! Hardware detection used to pick a kernel profile and sane partition defaults.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    None,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub cpu: CpuVendor,
    pub gpu: GpuVendor,
    pub ram_bytes: u64,
    pub is_laptop: bool,
}

impl Profile {
    /// Detects the current machine's profile via /proc/cpuinfo, lspci and DMI chassis-type.
    /// Detection failures degrade to `Unknown`/`None` rather than erroring — the caller
    /// always has a fallback path (manual override, or a `-generic` kernel package).
    pub fn detect() -> crate::Result<Self> {
        Ok(Self {
            cpu: detect_cpu_vendor(),
            gpu: detect_gpu_vendor(),
            ram_bytes: detect_ram_bytes(),
            is_laptop: detect_is_laptop(),
        })
    }

    /// Kernel package name suffix this profile should match against in the store,
    /// e.g. "intel-nvidia". Falls back to "generic" per-field when a component is unknown.
    pub fn kernel_suffix(&self) -> String {
        let cpu = match self.cpu {
            CpuVendor::Intel => "intel",
            CpuVendor::Amd => "amd",
            CpuVendor::Unknown => "generic",
        };
        let gpu = match self.gpu {
            GpuVendor::Nvidia => "nvidia",
            GpuVendor::Amd => "amd",
            GpuVendor::Intel => "intel",
            GpuVendor::None | GpuVendor::Unknown => "generic",
        };
        format!("{cpu}-{gpu}")
    }
}

fn detect_cpu_vendor() -> CpuVendor {
    let Ok(info) = std::fs::read_to_string("/proc/cpuinfo") else {
        return CpuVendor::Unknown;
    };
    if info.contains("GenuineIntel") {
        CpuVendor::Intel
    } else if info.contains("AuthenticAMD") {
        CpuVendor::Amd
    } else {
        CpuVendor::Unknown
    }
}

fn detect_gpu_vendor() -> GpuVendor {
    // TODO: parse `lspci -nnk` for VGA/3D controller vendor IDs (10de = Nvidia, 1002 = AMD, 8086 = Intel).
    GpuVendor::Unknown
}

fn detect_ram_bytes() -> u64 {
    let Ok(info) = std::fs::read_to_string("/proc/meminfo") else {
        return 0;
    };
    info.lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

fn detect_is_laptop() -> bool {
    std::fs::read_to_string("/sys/class/dmi/id/chassis_type")
        .map(|s| matches!(s.trim(), "8" | "9" | "10" | "14")) // Portable/Laptop/Notebook/Sub-Notebook
        .unwrap_or(false)
}
