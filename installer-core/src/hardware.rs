//! Hardware detection used to pick a kernel profile and sane partition defaults.
//!
//! The kernel store's combo vocabulary (mirrors `kernel-configs/{cpu,gpu,platform}/` in
//! the portage-store overlay, see `gen-popular-targets.py`) is three axes:
//! `<cpu>-<gpu>-<platform>`. EC/WMI vendor and modem support are *not* axes — every combo
//! is built with `ec/all` + `modem/all` (all vendor drivers as modules; `--skip-modules`
//! means they never affected the bzImage anyway, so splitting the matrix on them just
//! produced identical kernels under different names). `server` isn't a platform value
//! either — dropped as out of scope for this distro.
//!
//! Only the top-N most popular *valid* combinations are actually built (304 as of the
//! last matrix run, out of 18×7×3=378 possible), so exact matches aren't guaranteed —
//! `kernel::resolve` degrades through this struct's `candidates()` to find one that's
//! actually in the store.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuArch {
    IntelRaptorlake,
    IntelAlderlake,
    IntelMeteorlake,
    IntelArrowlake,
    IntelRocketlake,
    IntelIcelake,
    IntelSkylake,
    IntelHaswell,
    IntelIvybridge,
    IntelSandybridge,
    AmdZnver5,
    AmdZnver4,
    AmdZnver3,
    AmdZnver2,
    AmdZnver1,
    AmdBdver4,
    AmdBtver2,
    /// x86-64-v3 baseline (AVX2/BMI2/FMA) — safe fallback for any CPU that supports it
    /// but isn't one of the specifically-tuned codenames above.
    GenericX86_64V3,
    /// x86-64-v2 baseline (SSE4.2/POPCNT) — widest-compatibility fallback, and the
    /// deliberate landing spot for CPUs we can *detect* but won't *guess* an exact
    /// microarch for (see `detect_cpu_arch`'s doc comment on why a wrong specific
    /// guess is worse than a correct generic one here).
    GenericX86_64V2,
}

impl CpuArch {
    fn as_str(self) -> &'static str {
        match self {
            CpuArch::IntelRaptorlake => "intel-raptorlake",
            CpuArch::IntelAlderlake => "intel-alderlake",
            CpuArch::IntelMeteorlake => "intel-meteorlake",
            CpuArch::IntelArrowlake => "intel-arrowlake",
            CpuArch::IntelRocketlake => "intel-rocketlake",
            CpuArch::IntelIcelake => "intel-icelake",
            CpuArch::IntelSkylake => "intel-skylake",
            CpuArch::IntelHaswell => "intel-haswell",
            CpuArch::IntelIvybridge => "intel-ivybridge",
            CpuArch::IntelSandybridge => "intel-sandybridge",
            CpuArch::AmdZnver5 => "amd-znver5",
            CpuArch::AmdZnver4 => "amd-znver4",
            CpuArch::AmdZnver3 => "amd-znver3",
            CpuArch::AmdZnver2 => "amd-znver2",
            CpuArch::AmdZnver1 => "amd-znver1",
            CpuArch::AmdBdver4 => "amd-bdver4",
            CpuArch::AmdBtver2 => "amd-btver2",
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
    /// Intel Arc/Battlemage+ discrete GPUs (new `xe` kernel driver, distinct from `i915`).
    Xe,
    /// Open-source Nvidia driver — never auto-detected (hardware can't tell you which
    /// driver a user *wants*), only reachable as a fallback candidate if the proprietary
    /// `nvidia` combo isn't in the store.
    Nouveau,
    /// Pre-GCN AMD/ATI cards (`radeon` driver) — same reasoning as Nouveau: not
    /// auto-detected, only a fallback candidate.
    RadeonLegacy,
}

impl Gpu {
    /// Every value an installer can offer, detected-first ordering is the caller's job.
    pub const ALL: [Gpu; 7] = [Gpu::Nvidia, Gpu::Nouveau, Gpu::Amd, Gpu::Intel, Gpu::Xe, Gpu::RadeonLegacy, Gpu::None];

    pub fn display_name(self) -> &'static str {
        match self {
            Gpu::Nvidia => "NVIDIA — proprietary driver",
            Gpu::Nouveau => "NVIDIA — open-source (nouveau)",
            Gpu::Amd => "AMD (amdgpu)",
            Gpu::Intel => "Intel (i915)",
            Gpu::Xe => "Intel Arc (xe)",
            Gpu::RadeonLegacy => "AMD/ATI legacy (radeon)",
            Gpu::None => "No discrete driver / software rendering",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Gpu::Intel => "intel",
            Gpu::Nvidia => "nvidia",
            Gpu::Amd => "amd",
            Gpu::None => "none",
            Gpu::Xe => "xe",
            Gpu::Nouveau => "nouveau",
            Gpu::RadeonLegacy => "radeon-legacy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Laptop,
    Desktop,
    Handheld,
}

impl Platform {
    fn as_str(self) -> &'static str {
        match self {
            Platform::Laptop => "laptop",
            Platform::Desktop => "desktop",
            Platform::Handheld => "handheld",
        }
    }

    /// Handheld is real but scored low (see `gen-popular-targets.py`'s weights) and a
    /// laptop-profile kernel is a close enough match (same mobile power/thermal handling)
    /// if no handheld build exists for this cpu/gpu pair.
    fn degrade(self) -> Option<Platform> {
        match self {
            Platform::Handheld => Some(Platform::Laptop),
            Platform::Laptop | Platform::Desktop => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub cpu: CpuArch,
    pub gpu: Gpu,
    pub platform: Platform,
    pub ram_bytes: u64,
}

impl Profile {
    /// Detects the current machine's profile. Individual sub-detectors degrade to a
    /// conservative default (`None`/generic) rather than erroring — the caller always
    /// has a fallback path via `candidates()` and, ultimately, a manual override.
    pub fn detect() -> crate::Result<Self> {
        let raw = RawInfo::gather();
        let platform = detect_platform(&raw);
        let gpu = sanitize_gpu(platform, detect_gpu(&raw));
        Ok(Self {
            cpu: detect_cpu_arch(&raw),
            gpu,
            platform,
            ram_bytes: detect_ram_bytes(),
        })
    }

    /// Three-axis combo string matching the store's naming scheme, e.g.
    /// "intel-raptorlake-nvidia-laptop".
    pub fn combo(&self) -> String {
        format!("{}-{}-{}", self.cpu.as_str(), self.gpu.as_str(), self.platform.as_str())
    }

    /// Degrades from most to least specific: platform first (handheld -> laptop, since
    /// it's the narrowest axis by popularity weight), then cpu to a generic level, then
    /// gpu to none, ending at combos virtually guaranteed to exist in any reasonably
    /// sized store: `generic-x86-64-v2-none-{laptop,desktop}`.
    pub fn candidates(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut push = |cpu: CpuArch, gpu: Gpu, platform: Platform| {
            let gpu = sanitize_gpu(platform, gpu);
            let s = format!("{}-{}-{}", cpu.as_str(), gpu.as_str(), platform.as_str());
            if seen.insert(s.clone()) {
                out.push(s);
            }
        };

        push(self.cpu, self.gpu, self.platform);
        if let Some(p) = self.platform.degrade() {
            push(self.cpu, self.gpu, p);
        }
        push(CpuArch::GenericX86_64V3, self.gpu, self.platform);
        push(CpuArch::GenericX86_64V3, Gpu::None, self.platform);
        push(CpuArch::GenericX86_64V2, Gpu::None, self.platform);
        push(CpuArch::GenericX86_64V2, Gpu::None, Platform::Laptop);
        push(CpuArch::GenericX86_64V2, Gpu::None, Platform::Desktop);

        out
    }
}

/// Handheld implies gpu in {amd, none} in the matrix's `valid()` — anything else (rare:
/// handheld with Intel/Nvidia graphics) gets pulled to `amd`, the closer of the two
/// permitted values for a discrete-GPU handheld.
fn sanitize_gpu(platform: Platform, gpu: Gpu) -> Gpu {
    if platform == Platform::Handheld && !matches!(gpu, Gpu::Amd | Gpu::None) {
        Gpu::Amd
    } else {
        gpu
    }
}

struct RawInfo {
    cpuinfo: String,
    product_name: String,
    chassis_type: String,
    lspci: String,
}

impl RawInfo {
    fn gather() -> Self {
        Self {
            cpuinfo: std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default(),
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

/// Best-effort microarchitecture ID from `/proc/cpuinfo`'s "model name" string — there's
/// no clean libc/kernel API for "give me the marketing codename". Only guesses a specific
/// codename when the SKU pattern is unambiguous (current Intel Core i3/5/7/9 and Core
/// Ultra generations, current AMD Ryzen number ranges); anything else — including the
/// legacy AMD FX/APU families (bdver4/btver2), which have no reliable SKU-string
/// signature — falls back to the x86-64 psABI feature level. That's a deliberate choice:
/// a wrong *specific* guess risks a kernel built with instructions the CPU doesn't
/// support (illegal instruction crash at boot); a correct *generic* fallback never does.
fn detect_cpu_arch(raw: &RawInfo) -> CpuArch {
    let model_name = raw
        .cpuinfo
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split(':').nth(1))
        .unwrap_or_default()
        .trim();

    if let Some(arch) = intel_core_ultra_arch(model_name) {
        return arch;
    }
    if model_name.contains("Intel") {
        if let Some(gen) = intel_generation(model_name) {
            let has_g_suffix = model_name
                .rsplit_once('-')
                .map(|(_, sku)| sku.contains('G') && sku.chars().any(|c| c.is_ascii_digit()))
                .unwrap_or(false);
            match (gen, has_g_suffix) {
                (13, _) | (14, _) => return CpuArch::IntelRaptorlake,
                (12, _) => return CpuArch::IntelAlderlake,
                (11, _) => return CpuArch::IntelRocketlake,
                (10, true) => return CpuArch::IntelIcelake,
                (6..=10, _) => return CpuArch::IntelSkylake,
                (4, _) | (5, _) => return CpuArch::IntelHaswell,
                (3, _) => return CpuArch::IntelIvybridge,
                (2, _) => return CpuArch::IntelSandybridge,
                _ => {}
            }
        }
    }
    if model_name.contains("AMD Ryzen") {
        // Ryzen model numbers encode generation in the leading digit(s). Good enough for
        // the mainstream desktop/laptop SKUs this store targets.
        if let Some(num) = first_number(model_name) {
            match num {
                9000..=9999 => return CpuArch::AmdZnver5,
                7000..=8999 => return CpuArch::AmdZnver4,
                5000..=6999 => return CpuArch::AmdZnver3,
                3000..=4999 => return CpuArch::AmdZnver2,
                1000..=2999 => return CpuArch::AmdZnver1,
                _ => {}
            }
        }
    }

    if x86_64_v3_supported(&raw.cpuinfo) {
        CpuArch::GenericX86_64V3
    } else {
        CpuArch::GenericX86_64V2
    }
}

/// Meteor Lake / Arrow Lake use "Core(TM) Ultra N NNNsuffix" naming (e.g.
/// "Core(TM) Ultra 7 155H") instead of the "iX-NNNNN" pattern — the leading digit of the
/// 3-digit model number is the generation: 1xx = Meteor Lake (Core Ultra 1), 2xx = Arrow
/// Lake (Core Ultra 2). Matches on "Ultra" alone since "(TM)"/"(R)" markers between
/// "Core" and "Ultra" vary across kernel/vendor cpuinfo formatting.
fn intel_core_ultra_arch(model_name: &str) -> Option<CpuArch> {
    let idx = model_name.find("Ultra")?;
    // " 7 155H" -> trim the leading space before the tier digit.
    let rest = model_name[idx + "Ultra".len()..].trim_start();
    // Skip the tier digit ("5"/"7"/"9") and the whitespace after it to reach the model number.
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_whitespace())
        .skip_while(|c| c.is_ascii_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    match digits.chars().next()? {
        '1' => Some(CpuArch::IntelMeteorlake),
        '2' => Some(CpuArch::IntelArrowlake),
        _ => None,
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
/// both an iGPU and a dGPU (e.g. Intel + Nvidia laptop), the discrete GPU wins, since
/// that's the one that actually needs a matching kernel driver profile. Intel Arc/
/// Battlemage discrete GPUs (device IDs starting `56` or `e2`) map to `Xe` instead of
/// plain `Intel`, since they need the newer `xe` driver rather than `i915`.
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

    found.sort_by_key(|v| !matches!(v, Gpu::Nvidia | Gpu::Amd | Gpu::Xe));
    found.into_iter().next().unwrap_or(Gpu::None)
}

fn vendor_from_pci_line(line: &str) -> Option<Gpu> {
    if line.contains("[10de:") {
        Some(Gpu::Nvidia)
    } else if line.contains("[1002:") {
        Some(Gpu::Amd)
    } else if let Some(idx) = line.find("[8086:") {
        let device_id = &line[idx + "[8086:".len()..];
        if device_id.starts_with("56") || device_id.starts_with("e2") {
            Some(Gpu::Xe)
        } else {
            Some(Gpu::Intel)
        }
    } else {
        None
    }
}

/// Chassis-type-based laptop/desktop detection (SMBIOS System Enclosure `Type` values:
/// 8/9/10/14 = Portable/Laptop/Notebook/Sub-Notebook). Handheld gaming PCs don't have a
/// dedicated SMBIOS chassis type, so they're caught by matching known product names
/// instead — the same reason `gen-popular-targets.py` treats handheld as its own axis
/// rather than deriving it from chassis type. Server-class chassis types fall through to
/// Desktop, since `server` was dropped from the platform axis entirely (out of scope for
/// this distro) and a desktop-profile kernel is the closer match anyway.
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
        _ => Platform::Desktop,
    }
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
            ram_bytes: 0,
        };
        assert_eq!(p.combo(), "intel-raptorlake-nvidia-laptop");
    }

    #[test]
    fn candidates_degrade_platform_then_cpu_then_gpu() {
        let p = Profile {
            cpu: CpuArch::AmdZnver4,
            gpu: Gpu::Amd,
            platform: Platform::Handheld,
            ram_bytes: 0,
        };
        let c = p.candidates();
        assert_eq!(c[0], "amd-znver4-amd-handheld");
        assert!(c.contains(&"amd-znver4-amd-laptop".to_string()));
        assert!(c.contains(&"generic-x86-64-v3-amd-handheld".to_string()));
        assert!(c.contains(&"generic-x86-64-v3-none-handheld".to_string()));
        assert!(c.last().unwrap().starts_with("generic-x86-64-v2-none-"));
    }

    #[test]
    fn handheld_forces_gpu_constraint() {
        assert_eq!(sanitize_gpu(Platform::Handheld, Gpu::Nvidia), Gpu::Amd);
        assert_eq!(sanitize_gpu(Platform::Handheld, Gpu::None), Gpu::None);
        assert_eq!(sanitize_gpu(Platform::Desktop, Gpu::Nvidia), Gpu::Nvidia);
    }

    #[test]
    fn handheld_degrades_to_laptop() {
        assert_eq!(Platform::Handheld.degrade(), Some(Platform::Laptop));
        assert_eq!(Platform::Laptop.degrade(), None);
        assert_eq!(Platform::Desktop.degrade(), None);
    }

    #[test]
    fn intel_generation_parses_hx_sku() {
        assert_eq!(intel_generation("Intel(R) Core(TM) i7-14650HX"), Some(14));
        assert_eq!(intel_generation("Intel(R) Core(TM) i5-1240P"), Some(12));
    }

    #[test]
    fn core_ultra_naming_maps_to_meteor_or_arrow_lake() {
        assert_eq!(
            intel_core_ultra_arch("Intel(R) Core(TM) Ultra 7 155H"),
            Some(CpuArch::IntelMeteorlake)
        );
        assert_eq!(
            intel_core_ultra_arch("Intel(R) Core(TM) Ultra 9 285K"),
            Some(CpuArch::IntelArrowlake)
        );
    }
}
