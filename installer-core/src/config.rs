//! Runtime configuration shared by both frontends. The store isn't published under a
//! fixed URL yet (still a local overlay — see `~/portage-store-architecture.md`, no
//! `sync-uri` configured), so these come from the environment rather than being
//! hardcoded to a URL that doesn't exist yet.

pub struct StoreEnv {
    pub binhost_url: String,
    pub overlay_git_url: String,
    pub overlay_name: String,
    pub kernel_base_name: String,
    /// Git URL of the wm-configs preset repo `wm::install` clones — same "not published
    /// under a fixed URL yet" situation as the store itself.
    pub wm_configs_git_url: String,
}

impl StoreEnv {
    pub fn from_env() -> Result<Self, String> {
        let get = |key: &str| std::env::var(key).map_err(|_| format!("{key} is not set"));
        Ok(Self {
            binhost_url: get("GENTOO_STORE_BINHOST_URL")?,
            overlay_git_url: get("GENTOO_STORE_OVERLAY_URL")?,
            overlay_name: std::env::var("GENTOO_STORE_OVERLAY_NAME")
                .unwrap_or_else(|_| "localrepo".into()),
            kernel_base_name: std::env::var("GENTOO_KERNEL_BASE_NAME")
                .unwrap_or_else(|_| "gentoo-diy-kernel".into()),
            wm_configs_git_url: get("GENTOO_WM_CONFIGS_URL")?,
        })
    }
}
