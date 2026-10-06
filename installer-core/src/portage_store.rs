//! portage-store on the installed system.
//!
//! The GUI live image carries portage-store (the GTK4 app store for Portage, with its CLI companion and the root-owned helper
//! behind one doas rule). Like ustan, it is copied from the live system instead of compiled on the target. It is the graphical
//! side of Portage, so it is installed only when the target has libadwaita (a desktop was installed). The program goes under
//! `/usr/local`; the helper has to stay at `/usr/libexec/portage-store/` because the program and the doas rule name that path.

use std::path::{Path, PathBuf};

const BIN: &[&str] = &["usr/bin/portage-store", "usr/bin/portage-store-cli"];
const HELPERS: &[&str] = &[
    "usr/libexec/portage-store/priv-helper",
    "usr/libexec/portage-store/sandbox-build.sh",
];
const DATA: &[&str] = &[
    "usr/share/applications/io.github.tarilka0gg.PortageStore.desktop",
    "usr/share/icons/hicolor/scalable/apps/io.github.tarilka0gg.PortageStore.svg",
    "usr/share/metainfo/io.github.tarilka0gg.PortageStore.metainfo.xml",
];
const HELPER_PATH: &str = "/usr/libexec/portage-store/priv-helper";
const LOG_DIR: &str = "var/log/portage-store";

/// Is portage-store on the live system we are running from?
pub fn available(live_root: &Path) -> bool {
    live_root.join(BIN[0]).is_file()
}

/// Already copied to the target?
pub fn installed(target: &Path) -> bool {
    target.join("usr/local/bin/portage-store").is_file()
}

fn destination(target: &Path, rel: &str) -> PathBuf {
    if rel.starts_with("usr/libexec/") {
        target.join(rel)
    } else {
        target
            .join("usr/local")
            .join(rel.strip_prefix("usr/").unwrap_or(rel))
    }
}

/// GID of `portage` in the target's `/etc/group` (the log directory belongs to that group).
fn portage_gid(target: &Path) -> Option<u32> {
    std::fs::read_to_string(target.join("etc/group"))
        .ok()?
        .lines()
        .find_map(|l| {
            let mut f = l.split(':');
            (f.next()? == "portage").then(|| f.nth(1)?.parse().ok())?
        })
}

/// The one passwordless rule the helper is built around (the later rule wins in doas, so it goes last). Added to `etc/doas.conf`
/// only when that file exists, keeping its mode (`0400`).
async fn allow_helper(target: &Path) -> crate::Result<bool> {
    let conf = target.join("etc/doas.conf");
    let Ok(text) = tokio::fs::read_to_string(&conf).await else {
        return Ok(false);
    };
    let rule = format!("permit nopass :wheel cmd {HELPER_PATH}\n");
    if text.contains(&rule) {
        return Ok(false);
    }
    let mut new = text;
    if !new.ends_with('\n') && !new.is_empty() {
        new.push('\n');
    }
    new.push_str(&rule);
    tokio::fs::write(&conf, new).await?; // keeps the existing permission bits
    Ok(true)
}

/// Copies portage-store from `live_root` into `target`. Returns what was installed, empty when the live system has none or the
/// target has no GTK stack.
pub async fn install_from_live(live_root: &Path, target: &Path) -> crate::Result<Vec<String>> {
    if !available(live_root) || !crate::ustan::target_has_gtk(target) {
        return Ok(Vec::new());
    }
    let mut done = Vec::new();
    for rel in BIN.iter().chain(HELPERS).chain(DATA) {
        let from = live_root.join(rel);
        if !from.is_file() {
            continue;
        }
        let to = destination(target, rel);
        if let Some(dir) = to.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::copy(&from, &to).await?; // keeps the permission bits
        done.push(to.strip_prefix(target).unwrap_or(&to).display().to_string());
    }
    // The helper appends to an audit log; the directory is root:portage 0750 as in the ebuild.
    let log = target.join(LOG_DIR);
    tokio::fs::create_dir_all(&log).await?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o750))?;
    }
    if let Some(gid) = portage_gid(target) {
        let _ = std::os::unix::fs::chown(&log, Some(0), Some(gid));
    }
    if allow_helper(target).await? {
        done.push("etc/doas.conf (helper rule)".into());
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gentoo-installer-store-{tag}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, rel: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, rel).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn live(tag: &str) -> PathBuf {
        let l = tree(tag);
        for f in BIN.iter().chain(HELPERS) {
            put(&l, f, 0o755);
        }
        for f in DATA {
            put(&l, f, 0o644);
        }
        l
    }

    #[tokio::test]
    async fn a_target_without_gtk_gets_nothing() {
        let (l, t) = (live("l1"), tree("t1"));
        assert!(install_from_live(&l, &t).await.unwrap().is_empty());
        assert!(!installed(&t));
    }

    #[tokio::test]
    async fn the_program_goes_to_usr_local_and_the_helper_stays_where_it_is_named() {
        let (l, t) = (live("l2"), tree("t2"));
        put(&t, "usr/lib64/libadwaita-1.so.0", 0o755);
        install_from_live(&l, &t).await.unwrap();
        assert!(installed(&t));
        assert!(t.join("usr/local/bin/portage-store-cli").is_file());
        assert!(t.join("usr/libexec/portage-store/priv-helper").is_file());
        assert!(t
            .join("usr/local/share/applications/io.github.tarilka0gg.PortageStore.desktop")
            .is_file());
        assert!(t.join(LOG_DIR).is_dir());
    }

    #[tokio::test]
    async fn the_doas_rule_is_added_once_and_the_file_stays_private() {
        use std::os::unix::fs::PermissionsExt;
        let (l, t) = (live("l3"), tree("t3"));
        put(&t, "usr/lib64/libadwaita-1.so.0", 0o755);
        put(&t, "etc/doas.conf", 0o600);
        std::fs::write(t.join("etc/doas.conf"), "permit :wheel\n").unwrap();
        install_from_live(&l, &t).await.unwrap();
        install_from_live(&l, &t).await.unwrap();
        let text = std::fs::read_to_string(t.join("etc/doas.conf")).unwrap();
        assert_eq!(
            text,
            "permit :wheel\npermit nopass :wheel cmd /usr/libexec/portage-store/priv-helper\n"
        );
        let mode = std::fs::metadata(t.join("etc/doas.conf"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the mode of an existing file is kept");
    }

    #[tokio::test]
    async fn without_a_doas_conf_nothing_is_invented() {
        let (l, t) = (live("l4"), tree("t4"));
        put(&t, "usr/lib64/libadwaita-1.so.0", 0o755);
        install_from_live(&l, &t).await.unwrap();
        assert!(!t.join("etc/doas.conf").exists());
    }
}
