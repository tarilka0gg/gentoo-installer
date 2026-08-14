//! Picks a precompiled kernel binary package from the store that matches the detected
//! (or manually overridden) hardware profile. Only a popularity-ranked subset of the full
//! cpu×gpu×platform matrix is actually built (304 of 378 as of the last matrix run — see
//! `hardware::Profile::candidates`), so this degrades through progressively more generic
//! combos rather than requiring an exact match.

use crate::hardware::Profile;

#[derive(Debug, Clone)]
pub struct KernelPackage {
    /// Fully qualified atom in the store, e.g.
    /// "sys-kernel/mykernel-bin-intel-raptorlake-nvidia-laptop".
    pub atom: String,
    /// Which candidate in the degrade chain matched — 0 means an exact match, higher
    /// means the profile's platform/cpu/gpu got generalized to find a build that
    /// actually exists in the store.
    pub degraded_by: usize,
}

pub fn resolve(base_name: &str, profile: &Profile, available_atoms: &[String]) -> crate::Result<KernelPackage> {
    for (i, combo) in profile.candidates().into_iter().enumerate() {
        let atom = format!("sys-kernel/{base_name}-bin-{combo}");
        if available_atoms.iter().any(|a| a == &atom) {
            return Ok(KernelPackage { atom, degraded_by: i });
        }
    }
    Err(crate::Error::NoKernelProfile(profile.combo()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{CpuArch, Gpu, Platform};

    #[test]
    fn exact_match_wins_when_present() {
        let profile = Profile {
            cpu: CpuArch::IntelRaptorlake,
            gpu: Gpu::Nvidia,
            platform: Platform::Laptop,
            ram_bytes: 0,
        };
        let atoms = vec!["sys-kernel/mykernel-bin-intel-raptorlake-nvidia-laptop".to_string()];
        let pkg = resolve("mykernel", &profile, &atoms).unwrap();
        assert_eq!(pkg.degraded_by, 0);
    }

    #[test]
    fn falls_back_when_exact_combo_missing() {
        let profile = Profile {
            cpu: CpuArch::AmdZnver3,
            gpu: Gpu::Intel,
            platform: Platform::Handheld,
            ram_bytes: 0,
        };
        // Only the laptop-degraded combo exists in the store.
        let atoms = vec!["sys-kernel/mykernel-bin-amd-znver3-intel-laptop".to_string()];
        let pkg = resolve("mykernel", &profile, &atoms).unwrap();
        assert!(pkg.atom.ends_with("amd-znver3-intel-laptop"));
        assert!(pkg.degraded_by > 0);
    }

    #[test]
    fn errors_when_nothing_matches() {
        let profile = Profile {
            cpu: CpuArch::IntelRaptorlake,
            gpu: Gpu::Nvidia,
            platform: Platform::Laptop,
            ram_bytes: 0,
        };
        assert!(resolve("mykernel", &profile, &[]).is_err());
    }
}
