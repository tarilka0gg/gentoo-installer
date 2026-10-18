//! Optional software groups the user can tick in the installer, emerged in the target after
//! the desktop. They are the software mix-ins of the Simple Linux profiles (`profiles/`, compiled in through `slp-profiles`):
//! what each group installs, the USE flags and licences it needs, and which flavor ticks it by default are data there, not
//! code here. Each atom was checked to exist in the synced `gentoo`/`guru` trees; the ones whose only keyword is `~amd64` get an
//! accept-keywords line, everything else resolves on a stock `amd64` profile.
//!
//! Deliberately absent: Steam (needs the `steam-overlay`, a multilib profile and licence
//! acceptance — more than an unattended step should decide for the user) and Discord
//! (proprietary licence).

use crate::chroot_emerge::{
    bind_mount_chroot_dirs, ensure_network_resolves, ensure_portage_tree, unmount_chroot_dirs,
};
use crate::command::CommandRunner;
use slp_profiles::{Kind, RenderOptions, Repo, Selection, State};
use std::path::Path;
use std::sync::LazyLock;

/// The flavor an install gets when nothing else is asked for.
pub const DEFAULT_FLAVOR: &str = "desktop";

/// A software group (a mix-in of the profiles), as the installer's pages and steps use it.
pub struct Group {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Ticked by default in the default flavor (`GENTOO_INSTALLER_PACKAGES` / the GUI list start from these).
    pub default: bool,
    pub atoms: &'static [&'static str],
    /// Atoms from `atoms` that only have `~amd64` keywords.
    pub testing: &'static [&'static str],
    /// `package.use` lines (`atom flag ...`) the atoms need on a stock stage3 profile —
    /// found by `emerge -p` on a real stage3, e.g. ghostty's `REQUIRED_USE` wants X or wayland.
    pub use_flags: &'static [&'static str],
    /// OpenRC services to switch on (default runlevel) once the atoms are installed, in order.
    pub services: &'static [&'static str],
    /// `package.license` lines (`atom license ...`) for atoms under a licence Portage does not accept
    /// by default (e.g. the redistributable firmware blobs).
    pub licenses: &'static [&'static str],
    /// Needs the GURU overlay (the desktop step always clones it).
    pub guru: bool,
    /// Only makes sense on a system with a desktop (the page hides it on a console flavor).
    pub needs_desktop: bool,
    /// What each package of the group is, for the Software page.
    pub apps: &'static [AppEntry],
}

/// One package of a group as the Software page shows it: a name, a sentence and, where there is one, a screenshot ([`screen`]).
pub struct AppEntry {
    pub atom: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub has_screen: bool,
}

/// The screenshot of a package of a group (JPEG, WebP or PNG bytes), compiled in with the profiles.
pub fn screen(group_id: &str, atom: &str) -> Option<&'static [u8]> {
    Repo::bundled().screen(group_id, atom)
}

fn leak_strs(v: &[String]) -> &'static [&'static str] {
    let items: Vec<&'static str> = v.iter().map(|s| &*Box::leak(s.clone().into_boxed_str())).collect();
    Box::leak(items.into_boxed_slice())
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Every software group, in the order they are offered. Built once from the profile data.
pub static GROUPS: LazyLock<Vec<Group>> = LazyLock::new(|| {
    let repo = Repo::bundled();
    let defaults = repo
        .info(Kind::Flavor, DEFAULT_FLAVOR)
        .map(|f| f.defaults)
        .unwrap_or_default();
    repo.software()
        .into_iter()
        .map(|m| Group {
            id: leak(&m.id),
            name: leak(&m.name),
            description: leak(&m.description),
            default: defaults.contains(&m.id),
            atoms: leak_strs(&m.atoms),
            testing: leak_strs(&m.testing),
            use_flags: leak_strs(&m.use_lines),
            services: leak_strs(&m.services),
            licenses: leak_strs(&m.licenses),
            guru: m.guru,
            needs_desktop: m.needs_desktop,
            apps: Box::leak(
                m.apps
                    .iter()
                    .map(|a| AppEntry {
                        atom: leak(&a.atom),
                        name: leak(&a.name),
                        description: leak(&a.description),
                        has_screen: a.has_screen,
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
        })
        .collect()
});

/// A flavor the installer offers: what kind of system, and what it ticks by default.
pub struct Flavor {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Whether the system gets a desktop.
    pub desktop: bool,
    /// The software groups ticked by default.
    pub defaults: Vec<String>,
}

/// The flavors to choose from, in order (the live image's own is not among them).
pub fn flavors() -> Vec<Flavor> {
    let repo = Repo::bundled();
    repo.flavors()
        .into_iter()
        .filter(|f| !f.hidden)
        .map(|f| {
            let desktop = flavor_has_desktop(&f.id);
            let defaults = f.defaults.into_iter().filter(|d| find(d).is_some()).collect();
            Flavor { id: f.id, name: f.name, description: f.description, desktop, defaults }
        })
        .collect()
}

/// Whether a flavor comes with a desktop (an unknown one: yes, as before flavors existed).
pub fn flavor_has_desktop(flavor: &str) -> bool {
    Repo::bundled()
        .resolve(&Selection { flavor: flavor.to_string(), mix_ins: Vec::new() })
        .map(|e| e.desktop)
        .unwrap_or(true)
}

pub fn find(id: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.id == id)
}

/// The ids ticked before the user touches anything (in the default flavor).
pub fn default_ids() -> Vec<String> {
    default_ids_for(DEFAULT_FLAVOR)
}

/// The ids a flavor ticks by default, in the order the groups are offered; empty for an unknown flavor.
pub fn default_ids_for(flavor: &str) -> Vec<String> {
    let wanted = Repo::bundled().info(Kind::Flavor, flavor).map(|f| f.defaults).unwrap_or_default();
    GROUPS
        .iter()
        .filter(|g| wanted.iter().any(|w| w == g.id))
        .map(|g| g.id.to_string())
        .collect()
}

/// Unknown ids are an error, not silently skipped: a typo in
/// `GENTOO_INSTALLER_PACKAGES` should not produce an install without the browser.
pub fn resolve(ids: &[String]) -> crate::Result<Vec<&'static Group>> {
    let mut out: Vec<&'static Group> = Vec::new();
    for id in ids {
        let group = find(id).ok_or_else(|| {
            let known: Vec<_> = GROUPS.iter().map(|g| g.id).collect();
            crate::Error::Other(anyhow::anyhow!(
                "unknown package group {id:?}; known: {}",
                known.join(", ")
            ))
        })?;
        if !out.iter().any(|g| g.id == group.id) {
            out.push(group);
        }
    }
    Ok(out)
}

/// Whether every atom of the chosen groups is already in the target's package database (`var/db/pkg`).
/// An empty selection counts as installed; an unknown id does not.
pub fn installed(target: &Path, ids: &[String]) -> bool {
    let Ok(groups) = resolve(ids) else {
        return false;
    };
    groups.iter().flat_map(|g| g.atoms.iter()).all(|atom| {
        let Some((category, name)) = atom.split_once('/') else {
            return false;
        };
        std::fs::read_dir(target.join("var/db/pkg").join(category))
            .map(|d| {
                d.filter_map(|e| e.ok()).any(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.strip_prefix(&format!("{name}-"))
                        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
                })
            })
            .unwrap_or(false)
    })
}

/// Emerges every atom of the chosen groups in one transaction (`--noreplace`, so a re-run
/// is a no-op), inside the usual chroot bootstrap. Nothing runs for an empty selection.
///
/// The Portage configuration they need is written through the profiles (`slp-profiles`): `package.use`, `package.accept_keywords`
/// and `package.license` as `slp`, the set `@slp` with the packages, and `/etc/slp/state` with the flavor, the groups and the
/// graphics cards (`gfx`: mix-in ids), so that `slp` on the installed system knows what the system was made from.
pub async fn install(
    runner: &dyn CommandRunner,
    target: &Path,
    flavor: &str,
    ids: &[String],
    gfx: &[String],
) -> crate::Result<()> {
    let chosen = resolve(ids)?;
    let groups: Vec<_> = chosen
        .iter()
        .copied()
        .filter(|g| crate::offline::group_available(g.id))
        .collect();
    if groups.is_empty() {
        return Ok(());
    }
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
    let selection = Selection {
        flavor: flavor.to_string(),
        mix_ins: groups.iter().map(|g| g.id.to_string()).collect(),
    };
    let effective = Repo::bundled()
        .resolve(&selection)
        .map_err(|e| crate::Error::Other(anyhow::anyhow!("{e}")))?;
    let io = |e: std::io::Error| crate::Error::Other(anyhow::anyhow!("writing the profile: {e}"));

    bind_mount_chroot_dirs(runner, target).await?;
    let result = async {
        ensure_network_resolves(target).await?;
        ensure_portage_tree(runner, target, target_str).await?;
        // Everything GURU ships is `~amd64` only, and so are the dependencies of its packages (mangohud wants imgui and implot, both
        // GURU): naming each atom misses them, and emerge stops with "masked by: ~amd64 keyword" before it builds anything
        // (autounmask does not lift this one). The overlay is cloned by the desktop step in every install, hence the wildcard.
        slp_profiles::render(target, &selection, &effective, RenderOptions { guru_wildcard: true }).map_err(io)?;
        let mut recorded = chosen.iter().map(|g| g.id.to_string()).collect::<Vec<_>>();
        let repo = Repo::bundled();
        for g in gfx {
            if repo.info(Kind::MixIn, g).is_some() && !recorded.contains(g) {
                recorded.push(g.clone());
            }
        }
        State(Selection { flavor: flavor.to_string(), mix_ins: recorded }).save(target).map_err(io)?;
        // A bare stage3 profile needs point USE changes for desktop software (harfbuzz for
        // freetype, nftables for iptables, X for vulkan-loader...), different for every
        // atom. `--autounmask-write --autounmask-continue` lets Portage write and apply
        // them; `CONFIG_PROTECT_MASK` makes it write `/etc/portage` directly instead of
        // `._cfg` files nobody would dispatch. Verified with `emerge -f` on a real stage3.
        let mut argv = vec![
            target_str,
            "env",
            "CONFIG_PROTECT_MASK=/etc/portage",
            "emerge",
            "--noreplace",
            "--autounmask-write",
            "--autounmask-continue",
        ];
        argv.extend(effective.atoms.iter().map(String::as_str));
        runner.run_status("chroot", &argv).await?;
        // Services only after the packages that ship their init scripts are really there.
        let services: Vec<&str> = effective.services.iter().map(String::as_str).collect();
        crate::services::enable_all(runner, target, &services).await
    }
    .await;
    unmount_chroot_dirs(runner, target).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    fn temp_target(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gentoo-installer-pkgs-{tag}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::create_dir_all(dir.join("var/db/repos/gentoo/profiles")).unwrap();
        std::fs::write(dir.join("etc/resolv.conf"), "nameserver 127.0.0.1\n").ok();
        dir
    }

    #[test]
    fn ids_are_unique_and_every_atom_is_category_slash_name() {
        let mut seen = std::collections::HashSet::new();
        for g in GROUPS.iter() {
            assert!(seen.insert(g.id), "duplicate group id {}", g.id);
            assert!(!g.atoms.is_empty());
            for a in g.atoms {
                assert!(
                    a.split('/').count() == 2 && !a.starts_with('=') && !a.contains(' '),
                    "{a}"
                );
            }
            for t in g.testing {
                assert!(
                    g.atoms.contains(t),
                    "{t} is marked testing but is not in {}",
                    g.id
                );
            }
        }
    }

    #[test]
    fn the_flavors_are_the_three_and_each_ticks_its_own_defaults() {
        let f = flavors();
        assert_eq!(f.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["minimal", "desktop", "workstation"]);
        assert_eq!(f.iter().map(|f| f.desktop).collect::<Vec<_>>(), [false, true, true]);
        assert_eq!(default_ids_for("minimal"), ["wifi", "tools", "disks", "hardware", "gentoo-tools"]);
        assert_eq!(
            default_ids(),
            ["wifi", "terminal", "browser", "tools", "files", "audio", "desktop-utils", "disks", "hardware", "gentoo-tools"]
        );
        assert!(default_ids_for("workstation").contains(&"dev".to_string()));
        assert!(default_ids_for("workstation").contains(&"terminal".to_string()), "a flavor keeps what it inherits");
        assert!(default_ids_for("no-such-flavor").is_empty());
        assert!(flavor_has_desktop("desktop") && !flavor_has_desktop("minimal"));
        assert!(flavor_has_desktop("no-such-flavor"), "unknown: a desktop, as before flavors existed");
        assert!(f.iter().all(|f| f.defaults.iter().all(|d| find(d).is_some())));
    }

    #[test]
    fn every_package_of_every_group_has_a_name_and_a_sentence() {
        let mut n = 0;
        for g in GROUPS.iter() {
            assert_eq!(g.apps.iter().map(|a| a.atom).collect::<Vec<_>>(), g.atoms, "{}: one entry per package, in order", g.id);
            for a in g.apps {
                assert!(!a.name.is_empty() && a.description.len() > 15, "{}: {}", g.id, a.atom);
                assert_eq!(a.has_screen, screen(g.id, a.atom).is_some());
                n += 1;
            }
        }
        assert!(n >= 69, "{n} packages described");
    }

    #[test]
    fn groups_that_need_a_desktop_are_marked() {
        let needs: Vec<&str> = GROUPS.iter().filter(|g| g.needs_desktop).map(|g| g.id).collect();
        assert_eq!(needs, ["terminal", "browser", "files", "audio", "desktop-utils", "media", "graphics", "chat", "office", "gaming"]);
    }

    #[test]
    fn unknown_group_is_rejected_before_anything_runs() {
        assert!(resolve(&["browser".into(), "browsr".into()]).is_err());
    }

    #[test]
    fn duplicates_collapse() {
        assert_eq!(resolve(&["dev".into(), "dev".into()]).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn empty_selection_runs_nothing() {
        let runner = FakeCommandRunner::new();
        install(&runner, Path::new("/nonexistent"), DEFAULT_FLAVOR, &[], &[])
            .await
            .unwrap();
        assert!(runner.calls().is_empty());
    }

    #[tokio::test]
    async fn emerges_all_atoms_in_one_call_and_unmounts() {
        let dir = temp_target("ok");
        let runner = FakeCommandRunner::new();
        install(&runner, &dir, DEFAULT_FLAVOR, &["browser".into(), "gaming".into()], &["gfx-amd".into()])
            .await
            .unwrap();

        let calls = runner.calls();
        let emerge = calls
            .iter()
            .find(|(c, a)| c == "chroot" && a.get(1).map(String::as_str) == Some("env"))
            .unwrap();
        assert_eq!(
            emerge.1[2..],
            [
                "CONFIG_PROTECT_MASK=/etc/portage",
                "emerge",
                "--noreplace",
                "--autounmask-write",
                "--autounmask-continue",
                "www-client/zen-bin",
                "games-util/gamemode",
                "games-util/mangohud"
            ]
        );
        assert_eq!(calls.iter().filter(|(c, _)| c == "umount").count(), 3);
        // what the system was made from is recorded for `slp`, and the packages are the set @slp
        let state = std::fs::read_to_string(dir.join("etc/slp/state")).unwrap();
        assert!(state.contains("flavor=desktop\nmix-in=browser\nmix-in=gaming\nmix-in=gfx-amd\n"), "{state}");
        let set = std::fs::read_to_string(dir.join("etc/portage/sets/slp")).unwrap();
        assert!(set.contains("www-client/zen-bin\ngames-util/gamemode\ngames-util/mangohud\n"), "{set}");
        let kw = std::fs::read_to_string(dir.join("etc/portage/package.accept_keywords/slp")).unwrap();
        assert!(
            kw.contains("games-util/gamemode ~amd64")
                && kw.contains("games-util/mangohud ~amd64")
                && kw.contains("www-client/zen-bin ~amd64")
                // the dependencies of GURU packages (mangohud -> imgui, implot) are GURU too
                && kw.contains("*/*::guru ~amd64")
                && !kw.contains("firefox"),
            "{kw}"
        );
        let uses = std::fs::read_to_string(dir.join("etc/portage/package.use/slp")).unwrap();
        assert!(
            uses.contains("games-util/gamemode elogind") && !uses.contains("ghostty"),
            "{uses}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn unmounts_even_when_emerge_fails() {
        let dir = temp_target("fail");
        let runner = FakeCommandRunner::new();
        runner.fail("chroot", "emerge: blocked");
        assert!(install(&runner, &dir, DEFAULT_FLAVOR, &["media".into()], &[]).await.is_err());
        assert_eq!(
            runner.calls().iter().filter(|(c, _)| c == "umount").count(),
            3
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod use_tests {
    use super::*;
    use crate::command::FakeCommandRunner;

    #[tokio::test]
    async fn terminal_group_turns_on_wayland_for_ghostty() {
        let dir =
            std::env::temp_dir().join(format!("gentoo-installer-pkgs-use-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("var/db/repos/gentoo/profiles")).unwrap();
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(dir.join("etc/resolv.conf"), "x\n").ok();

        install(&FakeCommandRunner::new(), &dir, DEFAULT_FLAVOR, &["terminal".into()], &[])
            .await
            .unwrap();

        let text = std::fs::read_to_string(dir.join("etc/portage/package.use/slp")).unwrap();
        assert!(text.contains("x11-terms/ghostty wayland"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_wifi_group_accepts_the_firmware_licence_and_enables_dbus_then_iwd() {
        let dir =
            std::env::temp_dir().join(format!("gentoo-installer-pkgs-wifi-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("var/db/repos/gentoo/profiles")).unwrap();
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(dir.join("etc/resolv.conf"), "x\n").ok();
        let runner = crate::command::FakeCommandRunner::new();

        install(&runner, &dir, DEFAULT_FLAVOR, &["wifi".into()], &[]).await.unwrap();

        let lic = std::fs::read_to_string(dir.join("etc/portage/package.license/slp")).unwrap();
        assert!(
            lic.contains("sys-kernel/linux-firmware linux-fw-redistributable"),
            "{lic}"
        );
        let calls = runner.calls();
        let rc: Vec<&str> = calls
            .iter()
            .filter(|(c, a)| c == "chroot" && a.get(1).map(String::as_str) == Some("rc-update"))
            .map(|(_, a)| a[3].as_str())
            .collect();
        assert_eq!(rc, ["dbus", "iwd"]);
        let emerge_at = calls
            .iter()
            .position(|(_, a)| a.iter().any(|x| x == "emerge"))
            .unwrap();
        let first_rc = calls
            .iter()
            .position(|(_, a)| a.iter().any(|x| x == "rc-update"))
            .unwrap();
        assert!(
            emerge_at < first_rc,
            "services are enabled only after the packages are installed"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn installed_looks_at_the_package_database_not_at_the_atoms_text() {
        let dir = std::env::temp_dir().join(format!("gi-pkgs-installed-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        assert!(installed(&dir, &[]), "nothing selected, nothing to install");
        assert!(!installed(&dir, &["browser".into()]));
        assert!(!installed(&dir, &["no-such-group".into()]));

        std::fs::create_dir_all(dir.join("var/db/pkg/www-client/zen-bin-1.23")).unwrap();
        assert!(installed(&dir, &["browser".into()]));
        // `zen-bin-1.23` must not satisfy a package that merely starts with the same letters.
        std::fs::create_dir_all(dir.join("var/db/pkg/app-editors")).unwrap();
        std::fs::create_dir_all(dir.join("var/db/pkg/app-editors/micro-extra-1")).unwrap();
        assert!(
            !installed(&dir, &["tools".into()]),
            "micro itself is not there"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
