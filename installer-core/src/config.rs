//! Runtime configuration shared by both frontends: where the kernels come from and where the desktop's configs are cloned from.
//!
//! Everything has a public default, so an image works with nothing set: the kernels are the assets of a GitHub release of
//! `simple-linux-binhost` (`DEFAULT_KERNELS_URL`), the configs come from `simple-linux-configs`. The environment variables are
//! overrides, for a local test store or a mirror. There is no overlay: what used to be one (a repository the installer cloned)
//! held nothing the install used; an overlay URL is still honoured when one is given.

/// The kernels (`Packages`, `<combo>.vmlinuz`, `<combo>-modules.tar.xz`, `<combo>-devel.tar.xz`) as the flat assets of a GitHub release.
pub const DEFAULT_KERNELS_URL: &str =
    "https://github.com/tarilka0gg/simple-linux-binhost/releases/download/kernels";
pub const DEFAULT_WM_CONFIGS_URL: &str = "https://github.com/tarilka0gg/simple-linux-configs.git";

pub struct StoreEnv {
    pub binhost_url: String,
    /// Empty: no overlay is cloned.
    pub overlay_git_url: String,
    pub overlay_name: String,
    pub kernel_base_name: String,
    /// Git URL of the wm-configs preset repo `wm::install` clones.
    pub wm_configs_git_url: String,
}

impl StoreEnv {
    /// Never fails any more (the name of the `Result` is kept for the frontends); every value has a default.
    pub fn from_env() -> Result<Self, String> {
        let get = |key: &str, default: &str| {
            std::env::var(key)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        Ok(Self {
            binhost_url: get("GENTOO_STORE_BINHOST_URL", DEFAULT_KERNELS_URL),
            overlay_git_url: std::env::var("GENTOO_STORE_OVERLAY_URL")
                .unwrap_or_default()
                .trim()
                .to_string(),
            overlay_name: get("GENTOO_STORE_OVERLAY_NAME", "localrepo"),
            kernel_base_name: get("GENTOO_KERNEL_BASE_NAME", "gentoo-diy-kernel"),
            wm_configs_git_url: get("GENTOO_WM_CONFIGS_URL", DEFAULT_WM_CONFIGS_URL),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_set_gives_the_public_defaults_and_no_overlay() {
        // Process-wide variables: this is the only test that touches them.
        for k in [
            "GENTOO_STORE_BINHOST_URL",
            "GENTOO_STORE_OVERLAY_URL",
            "GENTOO_WM_CONFIGS_URL",
        ] {
            std::env::remove_var(k);
        }
        let e = StoreEnv::from_env().unwrap();
        assert_eq!(e.binhost_url, DEFAULT_KERNELS_URL);
        assert_eq!(e.wm_configs_git_url, DEFAULT_WM_CONFIGS_URL);
        assert!(e.overlay_git_url.is_empty());
        std::env::set_var("GENTOO_STORE_BINHOST_URL", "  http://10.0.2.2:8123 ");
        assert_eq!(
            StoreEnv::from_env().unwrap().binhost_url,
            "http://10.0.2.2:8123"
        );
        std::env::set_var("GENTOO_STORE_BINHOST_URL", "");
        assert_eq!(
            StoreEnv::from_env().unwrap().binhost_url,
            DEFAULT_KERNELS_URL
        );
        std::env::remove_var("GENTOO_STORE_BINHOST_URL");
    }
}
