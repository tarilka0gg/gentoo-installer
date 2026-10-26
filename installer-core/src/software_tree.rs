//! The default desktop's software as one archive: the files that the packages of the desktop flavor's defaults (and the desktop
//! itself) add to the stage3, with their Portage records. It is built on the builder from the same binary packages, so a default
//! install unpacks it instead of merging each of its ~270 packages one by one on the target, which took most of the install.
//!
//! The archive carries only what does not depend on the machine: no `make.conf`, users, `fstab`, hostname, services or kernel (the
//! steps that follow write those as before). Two packages are not in it, and are emerged as before: `sys-kernel/linux-firmware`
//! (its files come from a distfile that is set per medium) and `www-client/zen-bin` (fetched from the network).

use crate::command::CommandRunner;
use crate::http;
use std::path::Path;

/// The archive's file name, on the binary host (the release asset) or in the offline payload's `software/` directory.
pub const ARCHIVE: &str = "software-desktop-default.tar.zst";

/// Atoms of the default set that the archive leaves out. They are emerged on the target.
pub const NOT_IN_ARCHIVE: &[&str] = &["sys-kernel/linux-firmware", "www-client/zen-bin"];

/// Where the archive is: a GitHub release is flat (its assets are the files), any other store has a `software/` directory.
pub fn url(binhost_url: &str) -> String {
    let base = binhost_url.trim_end_matches('/');
    if base.contains("/releases/download/") {
        format!("{base}/{ARCHIVE}")
    } else {
        format!("{base}/software/{ARCHIVE}")
    }
}

/// Whether this install is the one the archive was built for: the desktop flavor with its default groups (the ones an offline
/// medium can install), the default desktop, and nothing more. Anything else is merged package by package, as before.
pub fn applies(
    flavor: &str,
    groups: &[String],
    desktop_atoms: &[String],
    default_groups: &[String],
    default_atoms: &[String],
) -> bool {
    if flavor != "desktop" {
        return false;
    }
    let same = |a: &[String], b: &[String]| {
        let mut x: Vec<&String> = a.iter().collect();
        let mut y: Vec<&String> = b.iter().collect();
        x.sort();
        x.dedup();
        y.sort();
        y.dedup();
        x == y
    };
    same(groups, default_groups) && same(desktop_atoms, default_atoms)
}

/// Downloads the archive and unpacks it into `target`. `Ok(false)` when the archive is not there (the caller merges the packages
/// instead); an error when it is there but does not unpack.
pub async fn install(
    runner: &dyn CommandRunner,
    target: &Path,
    binhost_url: &str,
) -> crate::Result<bool> {
    let file = std::env::temp_dir().join(format!("gentoo-installer-{ARCHIVE}"));
    if http::download_to_file(&url(binhost_url), &file)
        .await
        .is_err()
    {
        tokio::fs::remove_file(&file).await.ok();
        return Ok(false);
    }
    let file_str = file
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 archive path")))?;
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::Error::Other(anyhow::anyhow!("non-utf8 target path")))?;
    let result = runner
        .run_status(
            "tar",
            &[
                "--zstd",
                "--numeric-owner",
                "-xpf",
                file_str,
                "-C",
                target_str,
            ],
        )
        .await;
    tokio::fs::remove_file(&file).await.ok();
    result?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_archive_is_flat_on_a_release_and_in_a_directory_elsewhere() {
        assert_eq!(
            url("https://github.com/o/r/releases/download/binhost"),
            "https://github.com/o/r/releases/download/binhost/software-desktop-default.tar.zst"
        );
        assert_eq!(
            url("http://127.0.0.1:4000/binhost/simple-linux/"),
            "http://127.0.0.1:4000/binhost/simple-linux/software/software-desktop-default.tar.zst"
        );
    }

    #[test]
    fn only_the_default_desktop_uses_the_archive() {
        let groups = s(&["terminal", "browser", "files"]);
        let atoms = s(&["gui-wm/niri", "gui-apps/noctalia", "sys-auth/seatd"]);
        assert!(applies("desktop", &groups, &atoms, &groups, &atoms));
        // a group more, or a different order of the same groups: the order does not matter, the set does
        assert!(applies(
            "desktop",
            &s(&["files", "browser", "terminal"]),
            &atoms,
            &groups,
            &atoms
        ));
        assert!(!applies(
            "desktop",
            &s(&["files", "steam"]),
            &atoms,
            &groups,
            &atoms
        ));
        assert!(!applies(
            "desktop",
            &groups,
            &s(&["gui-wm/sway", "gui-apps/noctalia", "sys-auth/seatd"]),
            &groups,
            &atoms
        ));
        assert!(!applies("minimal", &groups, &atoms, &groups, &atoms));
        assert!(!applies("desktop", &groups, &[], &groups, &atoms));
    }
}
