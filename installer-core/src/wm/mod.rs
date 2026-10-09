//! Installs a compositor + Noctalia shell on the target, with a working preset config
//! for the new user — the other place besides `gpu_driver` that this codebase still
//! runs `emerge` during install (same documented exception, same `chroot_emerge`
//! plumbing). Niri is the silent default (auto-detected-default shape, same as
//! `keyboard`/`timezone`): Advanced setup is the only way to reach any other choice.
//!
//! Noctalia natively supports Niri, Hyprland, Sway, Labwc, and MangoWC — confirmed
//! against the upstream project — which is exactly this module's `WmChoice` list.
//!
//! Every atom/overlay/keyword requirement below was live-verified against a real synced
//! Portage tree + GURU + hyproverlay during implementation (not assumed):
//! - `gui-wm/niri`, `gui-wm/mangowm`, `gui-apps/noctalia` — GURU overlay
//!   (`https://github.com/gentoo-mirror/guru.git`), all `~amd64`.
//! - `gui-wm/hyprland` — its own dedicated overlay, hyproverlay
//!   (`https://codeberg.org/hyproverlay/hyproverlay.git`), `~amd64`.
//! - `gui-wm/sway` — main tree, has a genuinely stable amd64 version (1.9-r1 at the
//!   time of writing); no override needed.
//! - `gui-wm/labwc` — main tree, `~amd64`.
//! - `gui-apps/noctalia`'s current 5.x releases ship with no `KEYWORDS` at all (not
//!   even `~amd64` — fully unkeyworded betas), so it needs `**`, not `~amd64`, matching
//!   what this exact machine's own working Noctalia install already needed (see
//!   `noctalia-v5-niri` memory). Its `dev-cpp/sdbus-c++` dependency needs `~amd64`.

use crate::chroot_emerge::{
    bind_mount_chroot_dirs, ensure_network_resolves, ensure_portage_tree, unmount_chroot_dirs,
    write_portage_entry,
};
use crate::command::CommandRunner;
use std::path::Path;

mod packages;
mod preset;
mod session;
#[cfg(test)]
mod tests;

use packages::*;
use preset::*;
use session::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WmChoice {
    #[default]
    Niri,
    Hyprland,
    Sway,
    Labwc,
    MangoWc,
    /// suckless-style: its config is a C header compiled in via Portage's `savedconfig`.
    Dwl,
}

impl WmChoice {
    /// All choices, in the order the Advanced-setup picker lists them — niri first,
    /// matching its role as the silent default.
    pub const ALL: [WmChoice; 6] = [
        WmChoice::Niri,
        WmChoice::Hyprland,
        WmChoice::Sway,
        WmChoice::Labwc,
        WmChoice::MangoWc,
        WmChoice::Dwl,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            WmChoice::Niri => "niri",
            WmChoice::Hyprland => "Hyprland",
            WmChoice::Sway => "Sway",
            WmChoice::Labwc => "Labwc",
            WmChoice::MangoWc => "MangoWC",
            WmChoice::Dwl => "dwl",
        }
    }
}

const GURU_URL: &str = "https://github.com/gentoo-mirror/guru.git";
const HYPROVERLAY_URL: &str = "https://codeberg.org/hyproverlay/hyproverlay.git";

struct WmSpec {
    /// Portage atom for the compositor itself (Noctalia is emerged alongside every
    /// choice, added separately below — it isn't part of this list).
    atom: &'static str,
    /// Extra overlay this atom needs beyond GURU (which is always cloned, since
    /// Noctalia itself lives there), or `None` if it's in the main tree.
    extra_overlay: Option<(&'static str, &'static str)>,
    /// `None` means the atom resolves to a real stable keyword already — no override
    /// needed.
    accept_keywords: Option<&'static str>,
    /// `dbus-run-session --`-prefixed launch command written into the new user's
    /// `.bash_profile` tty1-autostart line.
    launch_cmd: &'static str,
    /// Matches a subdirectory name in the wm-configs preset repo.
    preset_dir: &'static str,
    /// Real XDG config directory name the compositor itself expects — not always the
    /// same as `preset_dir`/the WM's own name (Hyprland reads `~/.config/hypr`, not
    /// `~/.config/hyprland`; MangoWC reads `~/.config/mango`, matching its upstream
    /// project/binary name "mango" rather than "mangowc").
    config_dest_dir: &'static str,
}

fn spec(choice: WmChoice) -> WmSpec {
    match choice {
        WmChoice::Niri => WmSpec {
            atom: "gui-wm/niri",
            extra_overlay: None,
            accept_keywords: Some("gui-wm/niri ~amd64"),
            launch_cmd: "niri --session",
            preset_dir: "niri",
            config_dest_dir: "niri",
        },
        WmChoice::Hyprland => WmSpec {
            atom: "gui-wm/hyprland",
            extra_overlay: Some(("hyproverlay", HYPROVERLAY_URL)),
            accept_keywords: Some("gui-wm/hyprland ~amd64"),
            launch_cmd: "Hyprland",
            preset_dir: "hyprland",
            config_dest_dir: "hypr",
        },
        WmChoice::Sway => WmSpec {
            atom: "gui-wm/sway",
            extra_overlay: None,
            accept_keywords: None,
            launch_cmd: "sway",
            preset_dir: "sway",
            config_dest_dir: "sway",
        },
        WmChoice::Labwc => WmSpec {
            atom: "gui-wm/labwc",
            extra_overlay: None,
            accept_keywords: Some("gui-wm/labwc ~amd64"),
            launch_cmd: "labwc",
            preset_dir: "labwc",
            config_dest_dir: "labwc",
        },
        WmChoice::MangoWc => WmSpec {
            atom: "gui-wm/mangowm",
            extra_overlay: None,
            accept_keywords: Some("gui-wm/mangowm ~amd64"),
            launch_cmd: "mangowc",
            preset_dir: "mangowc",
            config_dest_dir: "mango",
        },
        WmChoice::Dwl => WmSpec {
            atom: "gui-wm/dwl",
            extra_overlay: None,
            accept_keywords: Some("gui-wm/dwl ~amd64"),
            // dwl has no exec-on-startup of its own: Noctalia rides on its `-s` flag.
            launch_cmd: "dwl -s noctalia",
            preset_dir: "dwl",
            config_dest_dir: "dwl",
        },
    }
}

/// Emerges the chosen compositor + Noctalia, then applies the matching preset config
/// (compositor config, shared Noctalia config, tty1-autostart `.bash_profile`) to
/// `username`'s home directory. Requires `account::create` to have already run —
/// the home directory and `/etc/passwd`/`/etc/group` entries must exist.
pub async fn install(
    runner: &dyn CommandRunner,
    target: &Path,
    choice: WmChoice,
    configs_git_url: &str,
    username: &str,
    render: &crate::gpu::RenderPlan,
) -> crate::Result<()> {
    let spec = spec(choice);
    if configs_git_url.trim().is_empty() {
        return Err(crate::Error::Other(anyhow::anyhow!(
            "no wm-configs repository configured (GENTOO_WM_CONFIGS_URL); refusing to start a compile that would end in `git clone \"\"`"
        )));
    }

    let staging = std::env::temp_dir().join(format!(
        "gentoo-installer-wm-configs-{}",
        std::process::id()
    ));
    if staging.exists() {
        tokio::fs::remove_dir_all(&staging).await?;
    }
    let staging_str = staging
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 staging path")))?;
    // Cloned before the emerge: dwl's config is compiled in, so `config.h` has to be in
    // Portage's savedconfig directory by the time the build starts.
    runner
        .run_status(
            "git",
            &["clone", "--depth", "1", configs_git_url, staging_str],
        )
        .await?;

    let result = async {
        if choice == WmChoice::Dwl {
            install_savedconfig(&staging, target, &spec).await?;
        }
        emerge_wm_packages(runner, target, &spec).await?;
        enable_session_services(runner, target, username).await?;
        write_runtime_dir_script(target).await?;
        apply_preset_from_dir(runner, target, &staging, &spec, username, render).await
    }
    .await;
    tokio::fs::remove_dir_all(&staging).await.ok();
    result
}

/// Whether `choice`'s compositor binary is already in the target (what `DesktopPhase` checks on resume).
pub fn installed(target: &Path, choice: WmChoice) -> bool {
    let binary = spec(choice)
        .launch_cmd
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    !binary.is_empty() && target.join("usr/bin").join(binary).exists()
}
