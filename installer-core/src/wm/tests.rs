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

    let wm_config = tokio::fs::read_to_string(target_dir.join("home/tester/.config/niri/config"))
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
    let written = tokio::fs::read_to_string(target_dir.join("etc/portage/package.accept_keywords"))
        .await
        .unwrap();
    assert!(!written.contains("gui-wm/sway"));
    assert!(written.contains("gui-apps/noctalia **"));
    assert!(written.contains("dev-cpp/sdbus-c++ ~amd64"));

    tokio::fs::remove_dir_all(&target_dir).await.ok();
}

#[tokio::test]
async fn dwl_gets_its_config_h_as_savedconfig_and_launches_noctalia_with_dash_s() {
    let root = std::env::temp_dir().join(format!("gentoo-installer-dwl-{}", std::process::id()));
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
    let dir = std::env::temp_dir().join(format!("gentoo-installer-rundir-{}", std::process::id()));
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
    let snippet =
        std::fs::read_to_string(target.join("home/solomiya/.config/fish/conf.d/10-session.fish"))
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
    let mut t = "a = 1\nshared_gl_context     = true         # startup-only\nb = 2\n".to_string();
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

#[tokio::test]
async fn the_presets_fish_setup_is_copied_next_to_the_session_snippet_only_when_the_stage_has_fish()
{
    for with_fish in [true, false] {
        let tag = format!("fish{with_fish}-{}", std::process::id());
        let target = std::env::temp_dir().join(format!("gentoo-installer-fishcfg-t-{tag}"));
        let repo = std::env::temp_dir().join(format!("gentoo-installer-fishcfg-r-{tag}"));
        for d in [&target, &repo] {
            std::fs::remove_dir_all(d).ok();
        }
        std::fs::create_dir_all(target.join("home/tester")).unwrap();
        if with_fish {
            std::fs::create_dir_all(target.join("usr/bin")).unwrap();
            std::fs::write(target.join("usr/bin/fish"), "").unwrap();
        }
        make_preset_repo(&repo, "niri");
        std::fs::create_dir_all(repo.join("fish/conf.d")).unwrap();
        std::fs::write(repo.join("fish/config.fish"), "# preset\n").unwrap();
        std::fs::write(
            repo.join("fish/conf.d/tide-config.fish"),
            "set -g tide_x 1\n",
        )
        .unwrap();

        apply_preset_from_dir(
            &FakeCommandRunner::new(),
            &target,
            &repo,
            &spec(WmChoice::Niri),
            "tester",
            &Default::default(),
        )
        .await
        .unwrap();

        let fish = target.join("home/tester/.config/fish");
        assert_eq!(fish.join("config.fish").exists(), with_fish);
        assert_eq!(fish.join("conf.d/tide-config.fish").exists(), with_fish);
        // root gets the same setup (it has fish as its shell then)
        assert_eq!(
            target.join("root/.config/fish/config.fish").exists(),
            with_fish
        );
        assert!(
            !target
                .join("root/.config/fish/conf.d/10-session.fish")
                .exists(),
            "root must not auto-start the desktop"
        );
        // the session start-up file is written either way and the copy did not remove it
        assert_eq!(
            fish.join("conf.d/10-session.fish").exists(),
            with_fish,
            "only with fish in the stage"
        );
        std::fs::remove_dir_all(&target).ok();
        std::fs::remove_dir_all(&repo).ok();
    }
}
