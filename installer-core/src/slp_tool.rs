//! `slp` on the installed system.
//!
//! `slp` (flavors and mix-ins, like Funtoo's `epro`) is in the live image; like tui-shim and ustan it is copied from there into
//! `/usr/local/bin` of the target. It has no libraries to wait for: the profile data is compiled into it, so what it offers
//! always matches the installer that made the system (`/etc/slp/state` is what the installer recorded).

use std::path::Path;

const BIN: &str = "usr/local/bin/slp";

/// Is `slp` on the live system we are running from?
pub fn available(live_root: &Path) -> bool {
    live_root.join(BIN).is_file()
}

/// Already copied to the target?
pub fn installed(target: &Path) -> bool {
    target.join(BIN).is_file()
}

/// Copies `slp` from `live_root` into `target`. Returns what was installed (relative to `/usr/local`), empty when the live system
/// has none.
pub async fn install_from_live(live_root: &Path, target: &Path) -> crate::Result<Vec<String>> {
    if !available(live_root) {
        return Ok(Vec::new());
    }
    let to = target.join(BIN);
    tokio::fs::create_dir_all(to.parent().unwrap()).await?;
    tokio::fs::copy(live_root.join(BIN), &to).await?; // keeps the permission bits
    Ok(vec!["bin/slp".to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("gentoo-installer-slp-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn copied_when_the_live_system_has_it_and_not_otherwise() {
        use std::os::unix::fs::PermissionsExt;
        let (live, target) = (tree("live"), tree("target"));
        assert!(!available(&live));
        assert!(install_from_live(&live, &target).await.unwrap().is_empty());
        assert!(!installed(&target));
        let p = live.join(BIN);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "slp").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(install_from_live(&live, &target).await.unwrap(), ["bin/slp"]);
        assert!(installed(&target));
        assert_eq!(std::fs::metadata(target.join(BIN)).unwrap().permissions().mode() & 0o777, 0o755);
        std::fs::remove_dir_all(&live).ok();
        std::fs::remove_dir_all(&target).ok();
    }
}
