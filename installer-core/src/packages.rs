//! Optional software groups the user can tick in the installer, emerged in the target after
//! the desktop. Each atom was checked to exist in the synced `gentoo`/`guru` trees; the
//! ones whose only keyword is `~amd64` get an accept-keywords line, everything else
//! resolves on a stock `amd64` profile.
//!
//! Deliberately absent: Steam (needs the `steam-overlay`, a multilib profile and licence
//! acceptance — more than an unattended step should decide for the user) and Discord
//! (proprietary licence).

use crate::chroot_emerge::{
    bind_mount_chroot_dirs, ensure_network_resolves, ensure_portage_tree, unmount_chroot_dirs,
    write_portage_entry,
};
use crate::command::CommandRunner;
use std::path::Path;

pub struct Group {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Ticked by default (`GENTOO_INSTALLER_PACKAGES` / the GUI list start from these).
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
}

pub const GROUPS: &[Group] = &[
    Group {
        id: "wifi",
        name: "Wi-Fi and firmware",
        description: "iwd (Wi-Fi from the command line and Noctalia) and linux-firmware (Wi-Fi/GPU/Bluetooth blobs)",
        default: true,
        atoms: &["net-wireless/iwd", "sys-kernel/linux-firmware"],
        testing: &[],
        use_flags: &[],
        services: &["dbus", "iwd"],
        licenses: &["sys-kernel/linux-firmware linux-fw-redistributable no-source-code"],
        guru: false,
    },
    Group {
        id: "terminal",
        name: "Terminal",
        description: "ghostty, the terminal every WM preset binds to Mod+Return",
        default: true,
        atoms: &["x11-terms/ghostty"],
        testing: &[],
        use_flags: &["x11-terms/ghostty wayland"],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "browser",
        name: "Web browser",
        description: "Zen Browser (prebuilt binary from GURU; the same browser as in the live image)",
        default: true,
        atoms: &["www-client/zen-bin"],
        testing: &["www-client/zen-bin"],
        use_flags: &[],
        services: &[],
        licenses: &[],
        guru: true,
    },
    Group {
        id: "tools",
        name: "Command-line tools",
        description: "git, btop, fastfetch, fish, micro, tmux, screen, lsof, strace",
        default: true,
        atoms: &["dev-vcs/git", "sys-process/btop", "app-misc/fastfetch", "app-shells/fish", "app-editors/micro", "app-misc/tmux", "app-misc/screen", "sys-process/lsof", "dev-debug/strace"],
        testing: &["app-editors/micro"],
        use_flags: &[],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "files",
        name: "Files",
        description: "Thunar with thumbnails and gvfs (mounting drives and network places), as in the live image",
        default: true,
        atoms: &["xfce-base/thunar", "xfce-base/tumbler", "gnome-base/gvfs"],
        testing: &["xfce-base/thunar", "gnome-base/gvfs"],
        use_flags: &["gnome-base/gvfs udisks", "x11-libs/gtk+ X", "x11-libs/cairo X", "xfce-base/libxfce4ui wayland", "xfce-base/libxfce4windowing wayland", "xfce-base/xfce4-panel wayland"],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "audio",
        name: "Audio",
        description: "PipeWire with WirePlumber, ALSA and the sound server",
        default: true,
        atoms: &["media-video/pipewire", "media-video/wireplumber"],
        testing: &[],
        use_flags: &["media-video/pipewire sound-server pipewire-alsa"],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "desktop-utils",
        name: "Desktop utilities",
        description: "wl-clipboard, imv (images), the XDG portals, Adwaita icons, DejaVu fonts",
        default: true,
        atoms: &["gui-apps/wl-clipboard", "media-gfx/imv", "sys-apps/xdg-desktop-portal", "sys-apps/xdg-desktop-portal-gtk", "x11-themes/adwaita-icon-theme", "media-fonts/dejavu"],
        testing: &["x11-themes/adwaita-icon-theme"],
        use_flags: &[],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "disks",
        name: "Disk and rescue tools",
        description: "GParted, parted, btrfs/xfs/f2fs/exfat/ntfs/FAT tools, LVM, mdadm, cryptsetup, ddrescue, testdisk, smartmontools, nvme-cli",
        default: true,
        atoms: &["sys-block/gparted", "sys-block/parted", "sys-fs/btrfs-progs", "sys-fs/dosfstools", "sys-fs/exfatprogs", "sys-fs/f2fs-tools", "sys-fs/xfsprogs", "sys-fs/ntfs3g", "sys-fs/cryptsetup", "sys-fs/mdadm", "sys-fs/lvm2", "sys-fs/ddrescue", "app-admin/testdisk", "sys-apps/smartmontools", "sys-apps/nvme-cli", "sys-apps/hdparm", "sys-apps/gptfdisk"],
        testing: &[],
        use_flags: &["sys-fs/lvm2 lvm", "dev-cpp/gtkmm X", "dev-cpp/cairomm X", "x11-libs/gtk+ X", "x11-libs/cairo X"],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "hardware",
        name: "Hardware and network tools",
        description: "pciutils, usbutils, dmidecode, ethtool, nmap, tcpdump, traceroute, iw, wpa_supplicant",
        default: true,
        atoms: &["sys-apps/pciutils", "sys-apps/usbutils", "sys-apps/dmidecode", "sys-apps/ethtool", "net-analyzer/nmap", "net-analyzer/tcpdump", "net-analyzer/traceroute", "net-wireless/iw", "net-wireless/wpa_supplicant"],
        testing: &[],
        use_flags: &[],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "gentoo-tools",
        name: "Gentoo tools",
        description: "eix, gentoolkit (equery), cpuid2cpuflags",
        default: true,
        atoms: &["app-portage/eix", "app-portage/gentoolkit", "app-portage/cpuid2cpuflags"],
        testing: &[],
        use_flags: &[],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "dev",
        name: "Development",
        description: "neovim, Rust (rust-bin), podman",
        default: false,
        atoms: &["app-editors/neovim", "dev-lang/rust-bin", "app-containers/podman"],
        testing: &[],
        use_flags: &[],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "media",
        name: "Media",
        description: "mpv and VLC",
        default: false,
        atoms: &["media-video/mpv", "media-video/vlc"],
        testing: &[],
        // vlc[gui] wants Qt with OpenGL and QML (every Qt module the same: prebuilt ones that ask for -opengl cannot be mixed with it), mpv wants the Vulkan loader with X. Portage names these as "USE changes necessary", but
        // autounmask does not write them when the package that asks for them is a prebuilt one, so they are stated here.
        use_flags: &[
            "dev-qt/* opengl",
            "dev-qt/qt5compat qml",
            "kde-frameworks/sonnet qml",
            "media-libs/vulkan-loader X",
        ],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "graphics",
        name: "Graphics",
        description: "GIMP",
        default: false,
        atoms: &["media-gfx/gimp"],
        testing: &[],
        use_flags: &["app-text/poppler cairo", "media-libs/gegl cairo lcms"],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "chat",
        name: "Messaging",
        description: "Telegram Desktop (prebuilt)",
        default: false,
        atoms: &["net-im/telegram-desktop-bin"],
        testing: &["net-im/telegram-desktop-bin"],
        use_flags: &[],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "office",
        name: "Office",
        description: "LibreOffice (prebuilt)",
        default: false,
        atoms: &["app-office/libreoffice-bin"],
        testing: &[],
        use_flags: &[],
        services: &[],
        licenses: &[],
        guru: false,
    },
    Group {
        id: "gaming",
        name: "Gaming tools",
        description: "GameMode and MangoHud",
        default: false,
        atoms: &["games-util/gamemode", "games-util/mangohud"],
        testing: &["games-util/gamemode", "games-util/mangohud"],
        use_flags: &["games-util/gamemode elogind", "sys-apps/dbus elogind"],
        services: &[],
        licenses: &[],
        guru: true,
    },
];

pub fn find(id: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.id == id)
}

/// The ids ticked before the user touches anything.
pub fn default_ids() -> Vec<String> {
    GROUPS
        .iter()
        .filter(|g| g.default)
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
pub async fn install(
    runner: &dyn CommandRunner,
    target: &Path,
    ids: &[String],
) -> crate::Result<()> {
    let groups = resolve(ids)?;
    if groups.is_empty() {
        return Ok(());
    }
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;

    let mut atoms: Vec<&str> = groups
        .iter()
        .flat_map(|g| g.atoms.iter().copied())
        .collect();
    atoms.dedup();
    let mut testing = String::new();
    for atom in groups.iter().flat_map(|g| g.testing.iter()) {
        testing.push_str(&format!("{atom} ~amd64\n"));
    }
    // Everything GURU ships is `~amd64` only, and so are the dependencies of its packages (mangohud wants imgui and implot, both
    // GURU): naming each atom above misses them, and emerge stops with "masked by: ~amd64 keyword" before it builds anything
    // (autounmask does not lift this one). The overlay is cloned by the desktop step in every install.
    testing.push_str("*/*::guru ~amd64\n");

    bind_mount_chroot_dirs(runner, target).await?;
    let result = async {
        ensure_network_resolves(target).await?;
        ensure_portage_tree(runner, target, target_str).await?;
        let portage_dir = target.join("etc/portage");
        tokio::fs::create_dir_all(&portage_dir).await?;
        if !testing.is_empty() {
            write_portage_entry(
                &portage_dir.join("package.accept_keywords"),
                "gentoo-installer-packages",
                &testing,
            )
            .await?;
        }
        let use_lines: String = groups
            .iter()
            .flat_map(|g| g.use_flags.iter())
            .map(|l| format!("{l}\n"))
            .collect();
        if !use_lines.is_empty() {
            write_portage_entry(
                &portage_dir.join("package.use"),
                "gentoo-installer-packages",
                &use_lines,
            )
            .await?;
        }
        // A bare stage3 profile needs point USE changes for desktop software (harfbuzz for
        // freetype, nftables for iptables, X for vulkan-loader...), different for every
        // atom. `--autounmask-write --autounmask-continue` lets Portage write and apply
        // them; `CONFIG_PROTECT_MASK` makes it write `/etc/portage` directly instead of
        // `._cfg` files nobody would dispatch. Verified with `emerge -f` on a real stage3.
        let license_lines: String = groups
            .iter()
            .flat_map(|g| g.licenses.iter())
            .map(|l| format!("{l}\n"))
            .collect();
        if !license_lines.is_empty() {
            write_portage_entry(
                &portage_dir.join("package.license"),
                "gentoo-installer-packages",
                &license_lines,
            )
            .await?;
        }
        let mut argv = vec![
            target_str,
            "env",
            "CONFIG_PROTECT_MASK=/etc/portage",
            "emerge",
            "--noreplace",
            "--autounmask-write",
            "--autounmask-continue",
        ];
        argv.extend(atoms.iter().copied());
        runner.run_status("chroot", &argv).await?;
        // Services only after the packages that ship their init scripts are really there.
        let mut services: Vec<&str> = Vec::new();
        for s in groups.iter().flat_map(|g| g.services.iter().copied()) {
            if !services.contains(&s) {
                services.push(s);
            }
        }
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
        for g in GROUPS {
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
        install(&runner, Path::new("/nonexistent"), &[])
            .await
            .unwrap();
        assert!(runner.calls().is_empty());
    }

    #[tokio::test]
    async fn emerges_all_atoms_in_one_call_and_unmounts() {
        let dir = temp_target("ok");
        let runner = FakeCommandRunner::new();
        install(&runner, &dir, &["browser".into(), "gaming".into()])
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
        let kw = std::fs::read_to_string(dir.join("etc/portage/package.accept_keywords")).unwrap();
        assert!(
            kw.contains("games-util/gamemode ~amd64")
                && kw.contains("games-util/mangohud ~amd64")
                && kw.contains("www-client/zen-bin ~amd64")
                // the dependencies of GURU packages (mangohud -> imgui, implot) are GURU too
                && kw.contains("*/*::guru ~amd64")
                && !kw.contains("firefox"),
            "{kw}"
        );
        let uses = std::fs::read_to_string(dir.join("etc/portage/package.use")).unwrap();
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
        assert!(install(&runner, &dir, &["media".into()]).await.is_err());
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

        install(&FakeCommandRunner::new(), &dir, &["terminal".into()])
            .await
            .unwrap();

        let text = std::fs::read_to_string(dir.join("etc/portage/package.use")).unwrap();
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

        install(&runner, &dir, &["wifi".into()]).await.unwrap();

        let lic = std::fs::read_to_string(dir.join("etc/portage/package.license")).unwrap();
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
