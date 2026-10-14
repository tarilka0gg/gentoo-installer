//! tui-shim on the installed system.
//!
//! The GUI live image carries tui-shim (launchers for terminal programs, and the file-picker badge over micro and nvim). Like ustan it
//! is copied from the live system instead of compiled on the target, into `/usr/local/bin`. It is the GTK side of the desktop, so it is
//! copied only when the target has libadwaita and gtk4-layer-shell (the desktop step installs both). `tui-shim shims install` and
//! `sync --no-theme` are run per user by the fish configuration at the first prompt.

use std::path::Path;

const BIN: &str = "usr/bin/tui-shim";

/// Is tui-shim on the live system we are running from?
pub fn available(live_root: &Path) -> bool {
    live_root.join(BIN).is_file()
}

/// Already copied to the target?
pub fn installed(target: &Path) -> bool {
    target.join("usr/local/bin/tui-shim").is_file()
}

/// The picker links libadwaita and gtk4-layer-shell; without them the copied binary would not start.
pub fn target_ready(target: &Path) -> bool {
    let has = |lib: &str| ["usr/lib64", "usr/lib"].iter().any(|d| target.join(d).join(lib).exists());
    has("libadwaita-1.so.0") && has("libgtk4-layer-shell.so.0")
}

/// Copies tui-shim from `live_root` into `target`. Returns what was installed (relative to `/usr/local`), empty when the live system
/// has none or the target lacks the libraries.
pub async fn install_from_live(live_root: &Path, target: &Path) -> crate::Result<Vec<String>> {
    if !available(live_root) || !target_ready(target) {
        return Ok(Vec::new());
    }
    let to = target.join("usr/local/bin/tui-shim");
    tokio::fs::create_dir_all(to.parent().unwrap()).await?;
    tokio::fs::copy(live_root.join(BIN), &to).await?; // keeps the permission bits
    Ok(vec!["bin/tui-shim".to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("gentoo-installer-tuishim-{tag}-{}", std::process::id()));
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

    #[tokio::test]
    async fn copied_only_when_the_target_has_the_gtk_libraries() {
        use std::os::unix::fs::PermissionsExt;
        let (live, target) = (tree("live1"), tree("target1"));
        put(&live, BIN, 0o755);
        assert!(install_from_live(&live, &target).await.unwrap().is_empty());
        put(&target, "usr/lib64/libadwaita-1.so.0", 0o755);
        assert!(install_from_live(&live, &target).await.unwrap().is_empty(), "layer-shell still missing");
        put(&target, "usr/lib64/libgtk4-layer-shell.so.0", 0o755);
        assert_eq!(install_from_live(&live, &target).await.unwrap(), ["bin/tui-shim"]);
        install_from_live(&live, &target).await.unwrap(); // a second run is harmless
        assert!(installed(&target));
        let mode = std::fs::metadata(target.join("usr/local/bin/tui-shim")).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
    }

    #[tokio::test]
    async fn a_live_system_without_tui_shim_installs_nothing() {
        let (live, target) = (tree("live2"), tree("target2"));
        put(&target, "usr/lib64/libadwaita-1.so.0", 0o755);
        put(&target, "usr/lib64/libgtk4-layer-shell.so.0", 0o755);
        assert!(!available(&live));
        assert!(install_from_live(&live, &target).await.unwrap().is_empty());
    }
}
