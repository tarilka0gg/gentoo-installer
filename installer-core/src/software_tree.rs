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

/// The account lines the archive carries, relative to the target.
const ACCOUNTS: &str = "usr/lib/simple-linux/accounts";

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
    merge_accounts(target).await?;
    Ok(true)
}

/// The account lines the packages add (the groups and users of seatd, messagebus, ...) are in the archive, not in the account files
/// (those belong to the machine, and were written by the installer already). Each line whose name is not in the target yet is
/// appended; the lines are then taken out of the target.
async fn merge_accounts(target: &Path) -> crate::Result<()> {
    let dir = target.join(ACCOUNTS);
    for name in ["group", "gshadow", "passwd", "shadow"] {
        let Ok(add) = tokio::fs::read_to_string(dir.join(name)).await else {
            continue;
        };
        let dest = target.join("etc").join(name);
        let mut text = tokio::fs::read_to_string(&dest).await.unwrap_or_default();
        let have: std::collections::HashSet<String> = text
            .lines()
            .filter_map(|l| l.split(':').next().map(str::to_string))
            .collect();
        for line in add.lines() {
            let key = line.split(':').next().unwrap_or_default();
            if !key.is_empty() && !have.contains(key) {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(line);
                text.push('\n');
            }
        }
        tokio::fs::write(&dest, text).await?;
    }
    tokio::fs::remove_dir_all(&dir).await.ok();
    tokio::fs::remove_dir(target.join("usr/lib/simple-linux"))
        .await
        .ok();
    Ok(())
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

    #[tokio::test]
    async fn the_account_lines_of_the_archive_are_added_once_and_the_directory_is_removed() {
        let dir = std::env::temp_dir().join(format!("software-accounts-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::create_dir_all(dir.join(ACCOUNTS)).unwrap();
        std::fs::write(dir.join("etc/group"), "root:x:0:\ntester:x:1000:\n").unwrap();
        std::fs::write(dir.join("etc/passwd"), "root:x:0:0::/root:/bin/bash\n").unwrap();
        std::fs::write(
            dir.join(ACCOUNTS).join("group"),
            "seat:x:960:\ntester:x:1000:\n",
        )
        .unwrap();
        std::fs::write(
            dir.join(ACCOUNTS).join("passwd"),
            "seat:x:960:960::/:/bin/false\n",
        )
        .unwrap();
        merge_accounts(&dir).await.unwrap();
        let group = std::fs::read_to_string(dir.join("etc/group")).unwrap();
        assert_eq!(group, "root:x:0:\ntester:x:1000:\nseat:x:960:\n");
        let passwd = std::fs::read_to_string(dir.join("etc/passwd")).unwrap();
        assert!(passwd.contains("seat:x:960:960"), "{passwd}");
        assert!(!dir.join(ACCOUNTS).exists());
        std::fs::remove_dir_all(&dir).ok();
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
