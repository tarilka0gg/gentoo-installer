use super::*;

/// Puts the preset's `config.h` where `savedconfig.eclass` restores it from
/// (`/etc/portage/savedconfig/<category>/<name>`), so dwl builds with the preset's keybinds.
pub(super) async fn install_savedconfig(
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

/// Bind-mounts, bootstraps network/tree/overlays, writes portage overrides, and emerges
/// `spec.atom` + Noctalia — split out from `install` so tests can assert exact emerge
/// argv per `WmChoice` without needing `git` to actually clone anything (see
/// `apply_preset_from_dir` for the other half).
pub(super) async fn emerge_wm_packages(
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
pub(super) async fn ensure_overlay(
    runner: &dyn CommandRunner,
    target: &Path,
    name: &str,
    url: &str,
) -> crate::Result<()> {
    let repo_dir = target.join(format!("var/db/repos/{name}"));
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
    // Already there (a resumed run, or the pinned tree snapshot that carries GURU): registered above, nothing to clone.
    if repo_dir.exists() {
        return Ok(());
    }

    tokio::fs::create_dir_all(repo_dir.parent().expect("var/db/repos always has a parent")).await?;
    let repo_dir_str = repo_dir
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 overlay path")))?;
    runner
        .run_status("git", &["clone", "--depth", "1", url, repo_dir_str])
        .await?;
    Ok(())
}

pub(super) async fn configure_wm_portage_overrides(
    target: &Path,
    spec: &WmSpec,
) -> crate::Result<()> {
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
