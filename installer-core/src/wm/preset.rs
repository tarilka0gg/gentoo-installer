use super::*;

/// Copies the matching `<wm>/` + `noctalia/` subdirectories of an already-cloned preset
/// repo (`preset_root`) into the new user's home, renders `bash_profile.tmpl`, and fixes
/// ownership. Split out from `install` so tests can point it at a plain local directory
/// instead of relying on `FakeCommandRunner` to have actually run `git clone` (it never
/// does — it only records the call). `chroot ... chown` at the end fixes ownership to
/// the target's own uid/gid for `username` — the host and target don't necessarily agree
/// on uid numbers, so a plain host-side `chown` isn't safe here (same reasoning
/// `account::create` documents for why it uses `useradd -R` instead of chroot).
/// The wallpapers of the live image (its session runs as root); the installed system's palette is generated from them.
const LIVE_WALLPAPERS: &str = "/root/Pictures/Wallpapers/simple";

pub(super) async fn apply_preset_from_dir(
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
    // Starting configs of the programs whose colours Noctalia generates (ghostty, btop): its hooks only edit a config that exists.
    if preset_root.join("apps").is_dir() {
        copy_dir(&preset_root.join("apps"), &config_dir).await?;
    }
    // The palette comes from the wallpaper, so the system needs wallpapers: the live image's own set goes along.
    let live_wallpapers = Path::new(LIVE_WALLPAPERS);
    if live_wallpapers.is_dir() {
        copy_dir(live_wallpapers, &home.join("Pictures/Wallpapers/simple")).await?;
    }
    // The author's fish setup (tide prompt and its plugins), for a system whose stage has fish. It goes in before the session
    // snippet below, which adds its own file to the same conf.d.
    if preset_root.join("fish").is_dir() && target.join("usr/bin/fish").is_file() {
        copy_dir(&preset_root.join("fish"), &config_dir.join("fish")).await?;
        // root has fish as its shell as well (see `account::set_root_shell`) and the same setup; the installer runs as root, so
        // the files are root's already.
        let root_fish = target.join("root/.config/fish");
        copy_dir(&preset_root.join("fish"), &root_fish).await?;
    }
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
pub(super) fn set_toml_bool(text: &mut String, key: &str, value: bool) -> bool {
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

pub(super) fn choice_of(spec: &WmSpec) -> WmChoice {
    WmChoice::ALL
        .into_iter()
        .find(|c| self::spec(*c).atom == spec.atom)
        .unwrap_or_default()
}

/// Applies the GPU decision to the copied configs: niri's `render-drm-device` and Noctalia's GL context.
pub(super) async fn tune_for_gpu(
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

pub(super) fn copy_dir<'a>(
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
