use super::*;

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

pub(super) async fn write_runtime_dir_script(target: &Path) -> crate::Result<()> {
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
pub(super) fn with_runtime_dir_fallback(profile: &str) -> String {
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
pub(super) fn fish_session_snippet(launch: &str) -> String {
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

/// What a compositor session needs from the OS and the stage3 does not give: the system bus
/// (`dbus`), a seat manager (`seatd` — the compositor opens the GPU and input devices through it,
/// not as root) and the user in the groups that may talk to it and to the devices. Without this
/// the installed system boots fine and then the compositor cannot start.
pub(super) async fn enable_session_services(
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

/// The compositor-specific env prefix that pins the DRM device (wlroots: `WLR_DRM_DEVICES`, Hyprland:
/// `AQ_DRM_DEVICES`). niri takes it from its config file instead (see [`tune_for_gpu`]).
pub(super) fn launch_with_gpu(launch: &str, render: &crate::gpu::RenderPlan) -> String {
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
