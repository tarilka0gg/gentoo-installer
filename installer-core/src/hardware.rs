//! Hardware detection used to pick a kernel profile and sane partition defaults.
//!
//! The kernel store's combo vocabulary (mirrors `kernel-configs/{cpu,gpu,platform,ec,modem}/`
//! in the portage-store overlay, see `gen-popular-targets.py`) is five axes:
//! `<cpu>-<gpu>-<platform>-<ec>-<modem>`. Only the top-N most popular *valid* combinations
//! are actually built (147 as of the last matrix run, out of 1728 possible), so exact
//! matches are the exception rather than the rule — `kernel::resolve` degrades through
//! this struct's `candidates()` to find one that's actually in the store.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuArch {
    IntelRaptorlake,
    IntelAlderlake,
    AmdZnver4,
    AmdZnver3,
    /// x86-64-v3 baseline (AVX2/BMI2/FMA) — safe fallback for any CPU that supports it
    /// but isn't one of the specifically-tuned codenames above.
    GenericX86_64V3,
    /// x86-64-v2 baseline (SSE4.2/POPCNT) — widest-compatibility fallback.
    GenericX86_64V2,
}

impl CpuArch {
    fn as_str(self) -> &'static str {
        match self {
            CpuArch::IntelRaptorlake => "intel-raptorlake",
            CpuArch::IntelAlderlake => "intel-alderlake",
            CpuArch::AmdZnver4 => "amd-znver4",
            CpuArch::AmdZnver3 => "amd-znver3",
            CpuArch::GenericX86_64V3 => "generic-x86-64-v3",
            CpuArch::GenericX86_64V2 => "generic-x86-64-v2",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gpu {
    Intel,
    Nvidia,
    Amd,
    None,
}

impl Gpu {
    fn as_str(self) -> &'static str {
        match self {
            Gpu::Intel => "intel",
            Gpu::Nvidia => "nvidia",
            Gpu::Amd => "amd",
            Gpu::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Laptop,
    Desktop,
    Server,
    Handheld,
}

impl Platform {
    fn as_str(self) -> &'static str {
        match self {
            Platform::Laptop => "laptop",
            Platform::Desktop => "desktop",
            Platform::Server => "server",
            Platform::Handheld => "handheld",
        }
    }

    /// Server/handheld are real platform values in the matrix but scored so low
    /// (see `gen-popular-targets.py`'s weights) that neither made the top-147 build.
    /// A server boots fine on a desktop-profile kernel (superset of ACPI/thermal
    /// handling, just with unused mobile bits); a handheld is close enough to laptop.
    fn degrade(self) -> Option<Platform> {
        match self {
            Platform::Server => Some(Platform::Desktop),
            Platform::Handheld => Some(Platform::Laptop),
            Platform::Laptop | Platform::Desktop => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ec {
    Lenovo,
    Hp,
    Dell,
    Asus,
    System76,
    None,
}

impl Ec {
    fn as_str(self) -> &'static str {
        match self {
            Ec::Lenovo => "lenovo",
            Ec::Hp => "hp",
            Ec::Dell => "dell",
            Ec::Asus => "asus",
            Ec::System76 => "system76",
            Ec::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modem {
    /// PCIe/SoC modem (Snapdragon X-style MHI transport).
    MhiSoc,
    UsbWwanGeneric,
    None,
}

impl Modem {
    fn as_str(self) -> &'static str {
        match self {
            Modem::MhiSoc => "mhi-soc",
            Modem::UsbWwanGeneric => "usb-wwan-generic",
            Modem::None => "none",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub cpu: CpuArch,
    pub gpu: Gpu,
    pub platform: Platform,
    pub ec: Ec,
    pub modem: Modem,
    pub ram_bytes: u64,
}

impl Profile {
    /// Detects the current machine's profile. Individual sub-detectors degrade to a
    /// conservative default (`None`/generic) rather than erroring — the caller always
    /// has a fallback path via `candidates()` and, ultimately, a manual override.
    pub fn detect() -> crate::Result<Self> {
        let raw = RawInfo::gather();
        let (platform, ec) = sanitize_platform_ec(detect_platform(&raw), detect_ec(&raw));
        let gpu = sanitize_gpu(platform, detect_gpu(&raw));
        Ok(Self {
            cpu: detect_cpu_arch(&raw),
            gpu,
            platform,
            ec,
            modem: detect_modem(&raw),
            ram_bytes: detect_ram_bytes(),
        })
    }

    /// Full five-axis combo string matching the store's naming scheme, e.g.
    /// "intel-raptorlake-nvidia-laptop-lenovo-none".
    pub fn combo(&self) -> String {
        format!(
            "{}-{}-{}-{}-{}",
            self.cpu.as_str(),
            self.gpu.as_str(),
            self.platform.as_str(),
            self.ec.as_str(),
            self.modem.as_str()
        )
    }

    /// Degrades from most to least specific, mirroring `gen-popular-targets.py`'s own
    /// `valid()` constraints (server implies ec=none, handheld implies ec in
    /// {none,asus} and gpu in {amd,none}) and its popularity weighting — narrow axes
    /// (modem, ec) degrade before broad ones (cpu, gpu). Yields the exact profile first,
    /// then progressively more generic combos, ending at a combo virtually guaranteed to
    /// exist in any reasonably-sized store: `generic-x86-64-v2-none-{laptop,desktop}-none-none`.
    pub fn candidates(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut push = |cpu: CpuArch, gpu: Gpu, platform: Platform, ec: Ec, modem: Modem| {
            let (platform, ec) = sanitize_platform_ec(platform, ec);
            let gpu = sanitize_gpu(platform, gpu);
            let s = format!(
                "{}-{}-{}-{}-{}",
                cpu.as_str(),
                gpu.as_str(),
                platform.as_str(),
                ec.as_str(),
                modem.as_str()
            );
            if seen.insert(s.clone()) {
                out.push(s);
            }
        };

        push(self.cpu, self.gpu, self.platform, self.ec, self.modem);
        push(self.cpu, self.gpu, self.platform, self.ec, Modem::None);
        push(self.cpu, self.gpu, self.platform, Ec::None, Modem::None);
        if let Some(p) = self.platform.degrade() {
            push(self.cpu, self.gpu, p, Ec::None, Modem::None);
        }
        push(CpuArch::GenericX86_64V3, self.gpu, self.platform, Ec::None, Modem::None);
        push(CpuArch::GenericX86_64V3, Gpu::None, self.platform, Ec::None, Modem::None);
        push(CpuArch::GenericX86_64V2, Gpu::None, self.platform, Ec::None, Modem::None);
        push(CpuArch::GenericX86_64V2, Gpu::None, Platform::Laptop, Ec::None, Modem::None);
        push(CpuArch::GenericX86_64V2, Gpu::None, Platform::Desktop, Ec::None, Modem::None);

        out
    }
}

/// Enforces the same cross-axis constraints as `gen-popular-targets.py`'s `valid()`:
/// server implies no vendor EC, handheld implies EC in {none, asus}. Applied both to
/// the detected profile and to every candidate generated during degradation, so we
/// never propose a combo the store could never contain.
fn sanitize_platform_ec(platform: Platform, ec: Ec) -> (Platform, Ec) {
    match platform {
        Platform::Server => (platform, Ec::None),
        Platform::Handheld if !matches!(ec, Ec::None | Ec::Asus) => (platform, Ec::None),
        _ => (platform, ec),
    }
}

/// Handheld implies gpu in {amd, none} in the matrix's `valid()`.
fn sanitize_gpu(platform: Platform, gpu: Gpu) -> Gpu {
    if platform == Platform::Handheld && !matches!(gpu, Gpu::Amd | Gpu::None) {
        Gpu::Amd
    } else {
        gpu
    }
}

struct RawInfo {
    cpuinfo: String,
    sys_vendor: String,
    product_name: String,
    chassis_type: String,
    lspci: String,
}

impl RawInfo {
    fn gather() -> Self {
        Self {
            cpuinfo: std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default(),
            sys_vendor: std::fs::read_to_string("/sys/class/dmi/id/sys_vendor")
                .unwrap_or_default()
                .trim()
                .to_string(),
            product_name: std::fs::read_to_string("/sys/class/dmi/id/product_name")
                .unwrap_or_default()
                .trim()
                .to_string(),
            chassis_type: std::fs::read_to_string("/sys/class/dmi/id/chassis_type")
                .unwrap_or_default()
                .trim()
                .to_string(),
            lspci: std::process::Command::new("lspci")
                .arg("-nn")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default(),
        }
    }
}

/// Best-effort microarchitecture ID from `/proc/cpuinfo`'s "model name" string, since
/// there's no clean libc/kernel API for "give me the marketing codename". Known-model
/// substring matches take priority; unmatched CPUs fall back to the x86-64 psABI
/// feature level computed from the advertised `flags`.
fn detect_cpu_arch(raw: &RawInfo) -> CpuArch {
    let model_name = raw
        .cpuinfo
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split(':').nth(1))
        .unwrap_or_default()
        .trim();

    // Older/mobile-focused kernel builds prefix the model name with "Nth Gen Intel" but
    // most (especially desktop-class/HX chips, e.g. "Intel(R) Core(TM) i7-14650HX") don't —
    // so match on the SKU's generation digits instead: for "i7-14650HX" the digits are
    // "14650", and the leading two digits (14) are the generation.
    if model_name.contains("Intel") {
        if let Some(gen) = intel_generation(model_name) {
            match gen {
                13 | 14 => return CpuArch::IntelRaptorlake,
                12 => return CpuArch::IntelAlderlake,
                _ => {}
            }
        }
    }
    if model_name.contains("AMD Ryzen") {
        // Ryzen model numbers encode generation in the leading digit(s): 7xxx/8xxx = Zen4,
        // 5xxx = Zen3. Good enough for the mainstream desktop/laptop SKUs this store targets;
        // genuinely ambiguous/rare SKUs fall through to the generic x86-64 level below.
        if let Some(num) = first_number(model_name) {
            if (7000..9000).contains(&num) {
                return CpuArch::AmdZnver4;
            }
            if (5000..6000).contains(&num) {
                return CpuArch::AmdZnver3;
            }
        }
    }

    if x86_64_v3_supported(&raw.cpuinfo) {
        CpuArch::GenericX86_64V3
    } else {
        CpuArch::GenericX86_64V2
    }
}

/// Extracts the generation from an Intel Core SKU string like "i7-14650HX" -> 14,
/// "i5-1240P" -> 12. The generation is always the leading two digits of the numeric
/// part, regardless of whether the full SKU number is 4 or 5 digits.
fn intel_generation(model_name: &str) -> Option<u32> {
    for prefix in ["i3-", "i5-", "i7-", "i9-"] {
        if let Some(idx) = model_name.find(prefix) {
            let rest = &model_name[idx + prefix.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 4 {
                return digits[..2].parse().ok();
            }
        }
    }
    None
}

fn first_number(s: &str) -> Option<u32> {
    let digits: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.len() >= 3 {
        digits.parse().ok()
    } else {
        None
    }
}

/// x86-64-v3 requires (per the psABI) avx, avx2, bmi1, bmi2, f16c, fma, lzcnt, movbe,
/// xsave on top of the v2 baseline — checked against the `flags` field of CPU0. The
/// kernel reports lzcnt support as `abm` (Advanced Bit Manipulation, which subsumes
/// lzcnt) rather than `lzcnt` itself, so either flag name satisfies that requirement.
fn x86_64_v3_supported(cpuinfo: &str) -> bool {
    let flags = cpuinfo
        .lines()
        .find(|l| l.starts_with("flags"))
        .unwrap_or_default();
    let present: std::collections::HashSet<&str> = flags.split_whitespace().collect();
    const REQUIRED: &[&[&str]] = &[
        &["avx"],
        &["avx2"],
        &["bmi1"],
        &["bmi2"],
        &["f16c"],
        &["fma"],
        &["lzcnt", "abm"],
        &["movbe"],
        &["xsave"],
    ];
    REQUIRED
        .iter()
        .all(|alternatives| alternatives.iter().any(|f| present.contains(f)))
}

/// Parses `lspci -nn` for VGA/3D/Display controller lines and matches the PCI vendor ID
/// (`[10de:xxxx]` = Nvidia, `[1002:xxxx]` = AMD, `[8086:xxxx]` = Intel). If a machine has
/// both an iGPU and a dGPU (e.g. Intel + Nvidia laptop), the discrete GPU wins, since that's
/// the one that actually needs a matching kernel driver profile.
fn detect_gpu(raw: &RawInfo) -> Gpu {
    let mut found: Vec<Gpu> = raw
        .lspci
        .lines()
        .filter(|l| {
            l.contains("VGA compatible controller")
                || l.contains("3D controller")
                || l.contains("Display controller")
        })
        .filter_map(vendor_from_pci_line)
        .collect();

    found.sort_by_key(|v| !matches!(v, Gpu::Nvidia | Gpu::Amd));
    found.into_iter().next().unwrap_or(Gpu::None)
}

fn vendor_from_pci_line(line: &str) -> Option<Gpu> {
    if line.contains("[10de:") {
        Some(Gpu::Nvidia)
    } else if line.contains("[1002:") {
        Some(Gpu::Amd)
    } else if line.contains("[8086:") {
        Some(Gpu::Intel)
    } else {
        None
    }
}

/// Chassis-type-based laptop/server detection (SMBIOS System Enclosure `Type` values:
/// 8/9/10/14 = Portable/Laptop/Notebook/Sub-Notebook, 17/23/28 = server-class chassis).
/// Handheld gaming PCs don't have a dedicated SMBIOS chassis type, so they're caught by
/// matching known product names instead — the same reason `gen-popular-targets.py`
/// treats handheld as its own axis rather than deriving it from chassis type.
fn detect_platform(raw: &RawInfo) -> Platform {
    const HANDHELD_PRODUCTS: &[&str] = &["ROG Ally", "Legion Go", "Steam Deck", "ONEXPLAYER", "GPD Win"];
    if HANDHELD_PRODUCTS
        .iter()
        .any(|p| raw.product_name.to_lowercase().contains(&p.to_lowercase()))
    {
        return Platform::Handheld;
    }

    match raw.chassis_type.as_str() {
        "8" | "9" | "10" | "14" => Platform::Laptop,
        "17" | "23" | "28" => Platform::Server,
        _ => Platform::Desktop,
    }
}

/// Matches `/sys/class/dmi/id/sys_vendor` against the store's supported EC vendors.
/// Everything else (or a chassis with no vendor-specific WMI/EC driver) maps to `none`.
fn detect_ec(raw: &RawInfo) -> Ec {
    let vendor = raw.sys_vendor.to_lowercase();
    if vendor.contains("lenovo") {
        Ec::Lenovo
    } else if vendor.contains("hp") || vendor.contains("hewlett") {
        Ec::Hp
    } else if vendor.contains("dell") {
        Ec::Dell
    } else if vendor.contains("asus") {
        Ec::Asus
    } else if vendor.contains("system76") {
        Ec::System76
    } else {
        Ec::None
    }
}

/// WWAN presence via `/sys/class/net/wwan*`; distinguishes the PCIe/MHI-SoC transport
/// from a USB WWAN dongle by checking which bus the backing device sits on.
fn detect_modem(_raw: &RawInfo) -> Modem {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Modem::None;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("wwan") {
            continue;
        }
        let subsystem = std::fs::read_link(entry.path().join("device/subsystem"))
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        return if subsystem == "pci" {
            Modem::MhiSoc
        } else {
            Modem::UsbWwanGeneric
        };
    }
    Modem::None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_matches_store_naming_scheme() {
        let p = Profile {
            cpu: CpuArch::IntelRaptorlake,
            gpu: Gpu::Nvidia,
            platform: Platform::Laptop,
            ec: Ec::Lenovo,
            modem: Modem::None,
            ram_bytes: 0,
        };
        assert_eq!(p.combo(), "intel-raptorlake-nvidia-laptop-lenovo-none");
    }

    #[test]
    fn candidates_degrade_modem_then_ec_then_cpu() {
        let p = Profile {
            cpu: CpuArch::AmdZnver4,
            gpu: Gpu::Amd,
            platform: Platform::Laptop,
            ec: Ec::System76,
            modem: Modem::UsbWwanGeneric,
            ram_bytes: 0,
        };
        let c = p.candidates();
        assert_eq!(c[0], "amd-znver4-amd-laptop-system76-usb-wwan-generic");
        assert!(c.contains(&"amd-znver4-amd-laptop-system76-none".to_string()));
        assert!(c.contains(&"amd-znver4-amd-laptop-none-none".to_string()));
        assert!(c.contains(&"generic-x86-64-v3-amd-laptop-none-none".to_string()));
        assert!(c.last().unwrap().starts_with("generic-x86-64-v2-none-"));
    }

    #[test]
    fn server_platform_forces_ec_none() {
        let (platform, ec) = sanitize_platform_ec(Platform::Server, Ec::Dell);
        assert_eq!(platform, Platform::Server);
        assert_eq!(ec, Ec::None);
    }

    #[test]
    fn handheld_forces_ec_and_gpu_constraints() {
        let (_, ec) = sanitize_platform_ec(Platform::Handheld, Ec::Dell);
        assert_eq!(ec, Ec::None);
        assert_eq!(sanitize_gpu(Platform::Handheld, Gpu::Nvidia), Gpu::Amd);
        assert_eq!(sanitize_gpu(Platform::Handheld, Gpu::None), Gpu::None);
    }

    #[test]
    fn server_degrades_to_desktop() {
        assert_eq!(Platform::Server.degrade(), Some(Platform::Desktop));
        assert_eq!(Platform::Handheld.degrade(), Some(Platform::Laptop));
        assert_eq!(Platform::Laptop.degrade(), None);
    }
}
