//! Picks a precompiled kernel binary package from the store that matches
//! the detected (or manually overridden) hardware profile, with a generic fallback.

use crate::hardware::Profile;

#[derive(Debug, Clone)]
pub struct KernelPackage {
    /// Fully qualified atom in the store, e.g. "sys-kernel/mykernel-bin-intel-nvidia".
    pub atom: String,
}

/// Naming scheme: `sys-kernel/<base>-bin-<cpu>-<gpu>`, falling back to
/// `sys-kernel/<base>-bin-generic` if no exact profile match exists in the store.
pub fn resolve(base_name: &str, profile: &Profile, available_atoms: &[String]) -> crate::Result<KernelPackage> {
    let exact = format!("sys-kernel/{base_name}-bin-{}", profile.kernel_suffix());
    if available_atoms.iter().any(|a| a == &exact) {
        return Ok(KernelPackage { atom: exact });
    }

    let generic = format!("sys-kernel/{base_name}-bin-generic");
    if available_atoms.iter().any(|a| a == &generic) {
        return Ok(KernelPackage { atom: generic });
    }

    Err(crate::Error::NoKernelProfile(profile.kernel_suffix()))
}
