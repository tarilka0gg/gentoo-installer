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

/// Puts the preset's `config.h` where `savedconfig.eclass` restores it from
/// (`/etc/portage/savedconfig/<category>/<name>`), so dwl builds with the preset's keybinds.
async fn install_savedconfig(
    preset_root: &Path,
    target: &Path,
    spec: &WmSpec,
) -> crate::Result<()> {
    let name = spec.atom.rsplit('/').next().unwrap_or(spec.atom);
    let dest_dir = target.join("etc/portage/savedconfig/gui-wm");
    tokio::fs::create_dir_all(&dest_dir).await?;
    tokio::fs::copy(
        preset_root.join(spec.preset_dir).join("config.h"),
        dest_dir.join(name),
    )
    .await?;
    // Without the flag the eclass ignores the file (`-savedconfig` is the default).
    let portage_dir = target.join("etc/portage");
    write_portage_entry(
        &portage_dir.join("package.use"),
        "gentoo-installer-dwl",
        &format!("{} savedconfig\n", spec.atom),
    )
    .await?;
    Ok(())
}

/// Created at boot by `/etc/local.d`: `/run/user/<uid>` for every regular user.
///
/// Nothing else makes it on this system — no systemd, and no `elogind` (whose PAM module normally does) —
/// and a compositor cannot start without `XDG_RUNTIME_DIR`. Found by booting a finished install: the
/// directory did not exist. `local` runs `*.start` files from `/etc/local.d` in the default runlevel.
pub const RUNTIME_DIR_SCRIPT: &str = "#!/bin/sh
# Written by the installer: XDG_RUNTIME_DIR for regular users (no elogind or systemd here to create it).
awk -F: '$3 >= 1000 && $3 < 60000 { print $3 \":\" $4 }' /etc/passwd | while IFS=: read -r uid gid; do
    install -d -m 0700 -o \"$uid\" -g \"$gid\" \"/run/user/$uid\"
done
";

async fn write_runtime_dir_script(target: &Path) -> crate::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = target.join("etc/local.d");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("10-xdg-runtime.start");
    tokio::fs::write(&path, RUNTIME_DIR_SCRIPT).await?;
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).await?;
    Ok(())
}

/// If `/run/user/<uid>` is missing at login (the boot script has not run, or ran before the user
/// existed), use a private directory under `~/.cache` instead of failing to start the session.
fn with_runtime_dir_fallback(profile: &str) -> String {
    const EXPORT: &str = "export XDG_RUNTIME_DIR=/run/user/$(id -u)";
    const FALLBACK: &str = "[ -d \"$XDG_RUNTIME_DIR\" ] || { export XDG_RUNTIME_DIR=\"$HOME/.cache/xdg-runtime\"; mkdir -p -m 700 \"$XDG_RUNTIME_DIR\"; }";
    if profile.contains(EXPORT) {
        profile.replacen(EXPORT, &format!("{EXPORT}\n{FALLBACK}"), 1)
    } else {
        profile.to_string()
    }
}

/// Start the compositor from tty1 for a fish login shell, the counterpart of the template's
/// `.bash_profile` block (same launch command, same runtime-directory fallback).
fn fish_session_snippet(launch: &str) -> String {
    format!(
        "# Written by the installer: start the session on tty1 (fish does not read ~/.bash_profile).
if status is-login; and test (tty) = /dev/tty1; and not set -q WAYLAND_DISPLAY
    set -gx XDG_RUNTIME_DIR /run/user/(id -u)
    if not test -d $XDG_RUNTIME_DIR
        set -gx XDG_RUNTIME_DIR $HOME/.cache/xdg-runtime
        mkdir -p -m 700 $XDG_RUNTIME_DIR
    end
    exec {launch}
end
"
    )
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

/// What a compositor session needs from the OS and the stage3 does not give: the system bus
/// (`dbus`), a seat manager (`seatd` — the compositor opens the GPU and input devices through it,
/// not as root) and the user in the groups that may talk to it and to the devices. Without this
/// the installed system boots fine and then the compositor cannot start.
async fn enable_session_services(
    runner: &dyn CommandRunner,
    target: &Path,
    username: &str,
) -> crate::Result<()> {
    crate::services::enable_all(runner, target, &["dbus", "seatd"]).await?;
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
    runner
        .run_status(
            "chroot",
            &[
                target_str,
                "usermod",
                "-aG",
                "seat,video,input,audio,render",
                username,
            ],
        )
        .await
}

/// Bind-mounts, bootstraps network/tree/overlays, writes portage overrides, and emerges
/// `spec.atom` + Noctalia — split out from `install` so tests can assert exact emerge
/// argv per `WmChoice` without needing `git` to actually clone anything (see
/// `apply_preset_from_dir` for the other half).
async fn emerge_wm_packages(
    runner: &dyn CommandRunner,
    target: &Path,
    spec: &WmSpec,
) -> crate::Result<()> {
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;

    bind_mount_chroot_dirs(runner, target).await?;

    let result = async {
        ensure_network_resolves(target).await?;
        ensure_portage_tree(runner, target, target_str).await?;
        ensure_overlay(runner, target, "guru", GURU_URL).await?;
        if let Some((name, url)) = spec.extra_overlay {
            ensure_overlay(runner, target, name, url).await?;
        }
        configure_wm_portage_overrides(target, spec).await?;
        runner
            .run_status(
                "chroot",
                &[
                    target_str,
                    "emerge",
                    spec.atom,
                    "gui-apps/noctalia",
                    "sys-auth/seatd",
                ],
            )
            .await
    }
    .await;

    unmount_chroot_dirs(runner, target).await;
    result?;
    Ok(())
}

/// Same shape as `store::configure`'s overlay clone (repos.conf entry + host-side `git
/// clone`, no chroot needed for the clone itself) — generalized here since this module
/// needs it for up to two overlays (GURU always, plus a WM-specific one), not just the
/// distro's own.
async fn ensure_overlay(
    runner: &dyn CommandRunner,
    target: &Path,
    name: &str,
    url: &str,
) -> crate::Result<()> {
    let repo_dir = target.join(format!("var/db/repos/{name}"));
    if repo_dir.exists() {
        return Ok(());
    }
    let repos_conf_dir = target.join("etc/portage/repos.conf");
    tokio::fs::create_dir_all(&repos_conf_dir).await?;
    let repos_conf = format!(
        "[{name}]\n\
         location = /var/db/repos/{name}\n\
         sync-type = git\n\
         sync-uri = {url}\n\
         auto-sync = yes\n"
    );
    tokio::fs::write(repos_conf_dir.join(format!("{name}.conf")), repos_conf).await?;

    tokio::fs::create_dir_all(repo_dir.parent().expect("var/db/repos always has a parent")).await?;
    let repo_dir_str = repo_dir
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 overlay path")))?;
    runner
        .run_status("git", &["clone", "--depth", "1", url, repo_dir_str])
        .await?;
    Ok(())
}

async fn configure_wm_portage_overrides(target: &Path, spec: &WmSpec) -> crate::Result<()> {
    let portage_dir = target.join("etc/portage");
    tokio::fs::create_dir_all(&portage_dir).await?;

    let mut keywords = String::new();
    if let Some(line) = spec.accept_keywords {
        keywords.push_str(line);
        keywords.push('\n');
    }
    // Noctalia's 5.x releases ship with no KEYWORDS at all (see module doc comment) —
    // `**` accepts them; `dev-cpp/sdbus-c++` is its one real-tree dependency that needs
    // a plain ~amd64 bump.
    keywords.push_str("gui-apps/noctalia **\ndev-cpp/sdbus-c++ ~amd64\n");
    write_portage_entry(
        &portage_dir.join("package.accept_keywords"),
        "gentoo-installer-wm",
        &keywords,
    )
    .await?;

    // Live-tested: niri/hyprland/mangowm/labwc/noctalia each have a "-9999" live-git
    // ebuild carrying the exact same KEYWORDS as their numbered releases (confirmed by
    // reading each ebuild directly), and 9999 always sorts as the highest version — so
    // once any of them is keyword-accepted at all, Portage picks the live checkout over
    // the real release. Masking each -9999 explicitly, unconditionally (regardless of
    // which WmChoice is running), keeps every choice on a real numbered release instead
    // of a live-git build with unpredictable, untested state.
    write_portage_entry(
        &portage_dir.join("package.mask"),
        "gentoo-installer-wm",
        "=gui-wm/niri-9999\n=gui-wm/hyprland-9999\n=gui-wm/mangowm-9999\n=gui-wm/labwc-9999\n=gui-wm/dwl-9999\n=gui-apps/noctalia-9999\n",
    )
    .await?;

    // Live-tested: Noctalia's secret-service integration (gnome-keyring, a genuine
    // feature here, not an optional extra like nvidia-drivers' GUI tool was — this step
    // is installing an actual desktop) pulls in a GTK/X dependency chain the base
    // profile's default USE flags don't satisfy. Applying exactly what emerge's own
    // autounmask output named as blocking, same approach as the nvidia driver step's
    // libglvnd fix.
    write_portage_entry(
        &portage_dir.join("package.use"),
        "gentoo-installer-wm",
        ">=media-libs/freetype-2.14.3 harfbuzz\n>=app-crypt/gcr-3.41.2-r2 gtk\n>=x11-libs/cairo-1.18.4-r1 X\nsys-auth/seatd server builtin\n",
    )
    .await?;
    Ok(())
}

/// Copies the matching `<wm>/` + `noctalia/` subdirectories of an already-cloned preset
/// repo (`preset_root`) into the new user's home, renders `bash_profile.tmpl`, and fixes
/// ownership. Split out from `install` so tests can point it at a plain local directory
/// instead of relying on `FakeCommandRunner` to have actually run `git clone` (it never
/// does — it only records the call). `chroot ... chown` at the end fixes ownership to
/// the target's own uid/gid for `username` — the host and target don't necessarily agree
/// on uid numbers, so a plain host-side `chown` isn't safe here (same reasoning
/// `account::create` documents for why it uses `useradd -R` instead of chroot).
async fn apply_preset_from_dir(
    runner: &dyn CommandRunner,
    target: &Path,
    preset_root: &Path,
    spec: &WmSpec,
    username: &str,
    render: &crate::gpu::RenderPlan,
) -> crate::Result<()> {
    let home = target.join("home").join(username);
    let config_dir = home.join(".config");
    tokio::fs::create_dir_all(&config_dir).await?;

    copy_dir(
        &preset_root.join(spec.preset_dir),
        &config_dir.join(spec.config_dest_dir),
    )
    .await?;
    copy_dir(&preset_root.join("noctalia"), &config_dir.join("noctalia")).await?;
    tune_for_gpu(&config_dir, choice_of(spec), render).await?;

    let tmpl = tokio::fs::read_to_string(preset_root.join("bash_profile.tmpl")).await?;
    let launch = format!(
        "dbus-run-session -- {}",
        launch_with_gpu(spec.launch_cmd, render)
    );
    let rendered = tmpl.replace("{{LAUNCH_CMD}}", &launch);
    tokio::fs::write(
        home.join(".bash_profile"),
        with_runtime_dir_fallback(&rendered),
    )
    .await?;

    // The user's login shell is fish when the stage has it (a custom stage does), and fish does not read
    // `.bash_profile` — the first real install booted to a fish prompt on tty1 and never started niri.
    if target.join("usr/bin/fish").is_file() {
        let fish_dir = config_dir.join("fish/conf.d");
        tokio::fs::create_dir_all(&fish_dir).await?;
        tokio::fs::write(
            fish_dir.join("10-session.fish"),
            fish_session_snippet(&launch),
        )
        .await?;
    }

    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
    let home_in_target = format!("/home/{username}");
    runner
        .run_status(
            "chroot",
            &[
                target_str,
                "chown",
                "-R",
                &format!("{username}:{username}"),
                &home_in_target,
            ],
        )
        .await?;

    Ok(())
}

/// Edits a TOML-ish `key = value` line in place, keeping the trailing comment. `false` when the key is absent.
fn set_toml_bool(text: &mut String, key: &str, value: bool) -> bool {
    let mut found = false;
    let out: Vec<String> = text
        .lines()
        .map(|line| {
            let t = line.trim_start();
            let is_key = t.starts_with(key) && t[key.len()..].trim_start().starts_with('=');
            if is_key && !found {
                found = true;
                let comment = line
                    .find('#')
                    .map(|i| format!("  {}", &line[i..]))
                    .unwrap_or_default();
                format!("{key} = {value}{comment}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if found {
        *text = out.join("\n") + "\n";
    }
    found
}

/// The compositor-specific env prefix that pins the DRM device (wlroots: `WLR_DRM_DEVICES`, Hyprland:
/// `AQ_DRM_DEVICES`). niri takes it from its config file instead (see [`tune_for_gpu`]).
fn launch_with_gpu(launch: &str, render: &crate::gpu::RenderPlan) -> String {
    let Some(card) = &render.card_node else {
        return launch.to_string();
    };
    let var = match launch.split_whitespace().next() {
        Some("Hyprland") => "AQ_DRM_DEVICES",
        Some("sway" | "labwc" | "mangowc" | "dwl") => "WLR_DRM_DEVICES",
        _ => return launch.to_string(),
    };
    format!("env {var}={card} {launch}")
}

fn choice_of(spec: &WmSpec) -> WmChoice {
    WmChoice::ALL
        .into_iter()
        .find(|c| self::spec(*c).atom == spec.atom)
        .unwrap_or_default()
}

/// Applies the GPU decision to the copied configs: niri's `render-drm-device` and Noctalia's GL context.
async fn tune_for_gpu(
    config_dir: &Path,
    choice: WmChoice,
    render: &crate::gpu::RenderPlan,
) -> crate::Result<()> {
    if choice == WmChoice::Niri {
        if let Some(node) = &render.render_node {
            let path = config_dir.join("niri/config.kdl");
            if let Ok(mut kdl) = tokio::fs::read_to_string(&path).await {
                if !kdl.lines().any(|l| l.trim_start().starts_with("debug")) {
                    kdl.push_str(&format!("\n// Written by the installer: render on the preferred GPU.\ndebug {{\n    render-drm-device \"{node}\"\n}}\n"));
                    tokio::fs::write(&path, kdl).await?;
                }
            }
        }
    }
    if render.proprietary_nvidia {
        let path = config_dir.join("noctalia/config.toml");
        if let Ok(mut toml) = tokio::fs::read_to_string(&path).await {
            if !set_toml_bool(&mut toml, "shared_gl_context", false) {
                toml.push_str("\nshared_gl_context = false\n");
            }
            tokio::fs::write(&path, toml).await?;
        }
    }
    Ok(())
}

fn copy_dir<'a>(
    src: &'a Path,
    dest: &'a Path,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + 'a>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(dest).await?;
        let mut entries = tokio::fs::read_dir(src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());
            if file_type.is_dir() {
                copy_dir(&src_path, &dest_path).await?;
            } else {
                tokio::fs::copy(&src_path, &dest_path).await?;
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    fn make_preset_repo(root: &Path, wm_dir: &str) {
        std::fs::create_dir_all(root.join(wm_dir)).unwrap();
        std::fs::write(root.join(wm_dir).join("config"), "wm config\n").unwrap();
        std::fs::create_dir_all(root.join("noctalia")).unwrap();
        std::fs::write(
            root.join("noctalia").join("config.toml"),
            "noctalia config\n",
        )
        .unwrap();
        std::fs::write(root.join("bash_profile.tmpl"), "exec {{LAUNCH_CMD}}\n").unwrap();
    }

    #[tokio::test]
    async fn each_wm_choice_emerges_its_own_atom_plus_noctalia() {
        for (choice, atom) in [
            (WmChoice::Niri, "gui-wm/niri"),
            (WmChoice::Hyprland, "gui-wm/hyprland"),
            (WmChoice::Sway, "gui-wm/sway"),
            (WmChoice::Labwc, "gui-wm/labwc"),
            (WmChoice::MangoWc, "gui-wm/mangowm"),
            (WmChoice::Dwl, "gui-wm/dwl"),
        ] {
            let target_dir = std::env::temp_dir().join(format!(
                "gentoo-installer-wm-test-{choice:?}-{}",
                std::process::id()
            ));
            tokio::fs::create_dir_all(&target_dir).await.unwrap();
            let runner = FakeCommandRunner::new();

            emerge_wm_packages(&runner, &target_dir, &spec(choice))
                .await
                .unwrap();

            let calls = runner.calls();
            assert!(calls.iter().any(|(cmd, args)| cmd == "chroot"
                && args.contains(&"emerge".to_string())
                && args.contains(&atom.to_string())
                && args.contains(&"gui-apps/noctalia".to_string())));

            tokio::fs::remove_dir_all(&target_dir).await.ok();
        }
    }

    #[tokio::test]
    async fn copies_preset_config_into_the_new_users_home_and_chowns_it() {
        let target_dir = std::env::temp_dir().join(format!(
            "gentoo-installer-wm-test-copy-{}",
            std::process::id()
        ));
        tokio::fs::create_dir_all(target_dir.join("home/tester"))
            .await
            .unwrap();
        let repo_dir = std::env::temp_dir().join(format!(
            "gentoo-installer-wm-repo-copy-{}",
            std::process::id()
        ));
        make_preset_repo(&repo_dir, "niri");
        let runner = FakeCommandRunner::new();

        apply_preset_from_dir(
            &runner,
            &target_dir,
            &repo_dir,
            &spec(WmChoice::Niri),
            "tester",
            &Default::default(),
        )
        .await
        .unwrap();

        let wm_config =
            tokio::fs::read_to_string(target_dir.join("home/tester/.config/niri/config"))
                .await
                .unwrap();
        assert_eq!(wm_config, "wm config\n");
        let noctalia_config =
            tokio::fs::read_to_string(target_dir.join("home/tester/.config/noctalia/config.toml"))
                .await
                .unwrap();
        assert_eq!(noctalia_config, "noctalia config\n");
        let bash_profile = tokio::fs::read_to_string(target_dir.join("home/tester/.bash_profile"))
            .await
            .unwrap();
        assert_eq!(bash_profile, "exec dbus-run-session -- niri --session\n");

        let calls = runner.calls();
        assert!(calls.iter().any(|(cmd, args)| cmd == "chroot"
            && args.contains(&"chown".to_string())
            && args.contains(&"tester:tester".to_string())));

        tokio::fs::remove_dir_all(&target_dir).await.ok();
        tokio::fs::remove_dir_all(&repo_dir).await.ok();
    }

    #[tokio::test]
    async fn sway_needs_no_accept_keywords_override_but_noctalia_still_does() {
        let target_dir = std::env::temp_dir().join(format!(
            "gentoo-installer-wm-test-sway-{}",
            std::process::id()
        ));
        tokio::fs::create_dir_all(&target_dir).await.unwrap();
        let runner = FakeCommandRunner::new();

        emerge_wm_packages(&runner, &target_dir, &spec(WmChoice::Sway))
            .await
            .unwrap();

        // This synthetic target has no pre-existing package.accept_keywords directory
        // (real stage3 ships one — see chroot_emerge's own directory-vs-file tests), so
        // write_portage_entry falls to its "write straight to the path" branch here.
        let written =
            tokio::fs::read_to_string(target_dir.join("etc/portage/package.accept_keywords"))
                .await
                .unwrap();
        assert!(!written.contains("gui-wm/sway"));
        assert!(written.contains("gui-apps/noctalia **"));
        assert!(written.contains("dev-cpp/sdbus-c++ ~amd64"));

        tokio::fs::remove_dir_all(&target_dir).await.ok();
    }

    #[tokio::test]
    async fn dwl_gets_its_config_h_as_savedconfig_and_launches_noctalia_with_dash_s() {
        let root =
            std::env::temp_dir().join(format!("gentoo-installer-dwl-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let preset = root.join("preset");
        let target = root.join("target");
        make_preset_repo(&preset, "dwl");
        std::fs::write(preset.join("dwl/config.h"), "/* dwl */\n").unwrap();

        install_savedconfig(&preset, &target, &spec(WmChoice::Dwl))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("etc/portage/savedconfig/gui-wm/dwl")).unwrap(),
            "/* dwl */\n"
        );
        let uses = std::fs::read_to_string(target.join("etc/portage/package.use")).unwrap();
        assert!(uses.contains("gui-wm/dwl savedconfig"), "{uses}");

        apply_preset_from_dir(
            &FakeCommandRunner::new(),
            &target,
            &preset,
            &spec(WmChoice::Dwl),
            "solomiya",
            &Default::default(),
        )
        .await
        .unwrap();
        let profile = std::fs::read_to_string(target.join("home/solomiya/.bash_profile")).unwrap();
        assert_eq!(profile, "exec dbus-run-session -- dwl -s noctalia\n");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_session_gets_dbus_and_seatd_enabled_and_its_user_the_device_groups() {
        let runner = FakeCommandRunner::new();
        enable_session_services(&runner, Path::new("/mnt/gentoo"), "solomiya")
            .await
            .unwrap();
        runner.assert_call(
            0,
            "chroot",
            &["/mnt/gentoo", "rc-update", "add", "dbus", "default"],
        );
        runner.assert_call(
            1,
            "chroot",
            &["/mnt/gentoo", "rc-update", "add", "seatd", "default"],
        );
        runner.assert_call(
            2,
            "chroot",
            &[
                "/mnt/gentoo",
                "usermod",
                "-aG",
                "seat,video,input,audio,render",
                "solomiya",
            ],
        );
    }

    #[tokio::test]
    async fn an_empty_wm_configs_url_fails_before_anything_is_compiled() {
        let runner = FakeCommandRunner::new();
        let err = install(
            &runner,
            Path::new("/mnt/gentoo"),
            WmChoice::Niri,
            "  ",
            "solomiya",
            &Default::default(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("wm-configs"), "{err}");
        assert!(
            runner.calls().is_empty(),
            "nothing may run: {:?}",
            runner.calls()
        );
    }

    #[tokio::test]
    async fn the_boot_script_that_makes_the_runtime_dir_is_executable_and_targets_regular_users() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("gentoo-installer-rundir-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        write_runtime_dir_script(&dir).await.unwrap();
        let p = dir.join("etc/local.d/10-xdg-runtime.start");
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.starts_with("#!/bin/sh")
                && text.contains("/run/user/$uid")
                && text.contains(">= 1000"),
            "{text}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_bash_profile_gets_a_runtime_dir_fallback_right_after_the_template_export() {
        let tmpl = "export XDG_RUNTIME_DIR=/run/user/$(id -u)\n\nif [ \"$(tty)\" = \"/dev/tty1\" ]; then\n\texec dbus-run-session -- niri --session\nfi\n";
        let out = with_runtime_dir_fallback(tmpl);
        let export_at = out.find("export XDG_RUNTIME_DIR=/run/user").unwrap();
        let fallback_at = out.find(".cache/xdg-runtime").unwrap();
        let exec_at = out.find("exec dbus-run-session").unwrap();
        assert!(export_at < fallback_at && fallback_at < exec_at, "{out}");
        assert_eq!(
            with_runtime_dir_fallback("no export here\n"),
            "no export here\n"
        );
    }

    #[tokio::test]
    async fn a_stage_with_fish_gets_the_session_started_from_fish_too() {
        let root =
            std::env::temp_dir().join(format!("gentoo-installer-fishsess-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let (preset, target) = (root.join("preset"), root.join("target"));
        make_preset_repo(&preset, "niri");

        // Without fish: no fish snippet.
        apply_preset_from_dir(
            &FakeCommandRunner::new(),
            &target,
            &preset,
            &spec(WmChoice::Niri),
            "solomiya",
            &Default::default(),
        )
        .await
        .unwrap();
        assert!(!target.join("home/solomiya/.config/fish").exists());

        // With fish (a custom stage): the snippet exists and starts the same command.
        std::fs::create_dir_all(target.join("usr/bin")).unwrap();
        std::fs::write(target.join("usr/bin/fish"), "").unwrap();
        apply_preset_from_dir(
            &FakeCommandRunner::new(),
            &target,
            &preset,
            &spec(WmChoice::Niri),
            "solomiya",
            &Default::default(),
        )
        .await
        .unwrap();
        let snippet = std::fs::read_to_string(
            target.join("home/solomiya/.config/fish/conf.d/10-session.fish"),
        )
        .unwrap();
        assert!(
            snippet.contains("exec dbus-run-session -- niri --session"),
            "{snippet}"
        );
        assert!(
            snippet.contains("/dev/tty1") && snippet.contains("xdg-runtime"),
            "{snippet}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn noctalia_gl_context_is_switched_off_keeping_the_comment() {
        let mut t =
            "a = 1\nshared_gl_context     = true         # startup-only\nb = 2\n".to_string();
        assert!(set_toml_bool(&mut t, "shared_gl_context", false));
        assert_eq!(
            t,
            "a = 1\nshared_gl_context = false  # startup-only\nb = 2\n"
        );
        assert!(!set_toml_bool(&mut t, "missing", true));
    }

    #[test]
    fn the_launch_command_pins_the_gpu_for_each_compositor_family() {
        let plan = crate::gpu::RenderPlan {
            card_node: Some("/dev/dri/by-path/pci-0000:01:00.0-card".into()),
            ..Default::default()
        };
        assert_eq!(
            launch_with_gpu("Hyprland", &plan),
            "env AQ_DRM_DEVICES=/dev/dri/by-path/pci-0000:01:00.0-card Hyprland"
        );
        assert_eq!(
            launch_with_gpu("dwl -s noctalia", &plan),
            "env WLR_DRM_DEVICES=/dev/dri/by-path/pci-0000:01:00.0-card dwl -s noctalia"
        );
        assert_eq!(
            launch_with_gpu("niri --session", &plan),
            "niri --session",
            "niri uses its config file"
        );
        assert_eq!(launch_with_gpu("sway", &Default::default()), "sway");
    }

    #[tokio::test]
    async fn niri_and_noctalia_configs_follow_the_gpu_plan() {
        let dir =
            std::env::temp_dir().join(format!("gentoo-installer-gpu-tune-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("niri")).unwrap();
        std::fs::create_dir_all(dir.join("noctalia")).unwrap();
        std::fs::write(dir.join("niri/config.kdl"), "input {}\n").unwrap();
        std::fs::write(
            dir.join("noctalia/config.toml"),
            "shared_gl_context = true # x\n",
        )
        .unwrap();
        let plan = crate::gpu::RenderPlan {
            render_node: Some("/dev/dri/by-path/pci-0000:01:00.0-render".into()),
            card_node: None,
            proprietary_nvidia: true,
        };
        tune_for_gpu(&dir, WmChoice::Niri, &plan).await.unwrap();
        tune_for_gpu(&dir, WmChoice::Niri, &plan).await.unwrap(); // idempotent
        let kdl = std::fs::read_to_string(dir.join("niri/config.kdl")).unwrap();
        assert_eq!(kdl.matches("render-drm-device").count(), 1, "{kdl}");
        assert!(kdl.contains("pci-0000:01:00.0-render"));
        assert!(std::fs::read_to_string(dir.join("noctalia/config.toml"))
            .unwrap()
            .contains("shared_gl_context = false"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
