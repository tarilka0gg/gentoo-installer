//! ustan on the installed system.
//!
//! The Simple Linux live images carry ustan (the Windows-style installer for .deb, AppImage, Flatpak and .exe). Compiling it on the
//! target would need Rust and Zig there, so the installer copies the finished files from the live system into `/usr/local` of the
//! target, where nothing owned by Portage lives. The command-line tool needs only libc and is always copied; the GTK window and its
//! menu entry only when the target has libadwaita (a desktop was installed), since the copy was linked against that.

use std::path::{Path, PathBuf};

const CLI: &str = "usr/bin/ustan";
const GUI: &str = "usr/bin/ustan-gui";
const GUI_FILES: &[&str] = &[
    "usr/share/applications/io.github.tarilka0gg.Ustan.desktop",
    "usr/share/icons/hicolor/scalable/apps/ustan.svg",
    "usr/share/metainfo/io.github.tarilka0gg.Ustan.metainfo.xml",
];

/// Is ustan on the live system we are running from (a plain dev machine has none)?
pub fn available(live_root: &Path) -> bool {
    live_root.join(CLI).is_file()
}

/// Already copied to the target?
pub fn installed(target: &Path) -> bool {
    target.join("usr/local/bin/ustan").is_file()
}

/// libadwaita is what `ustan-gui` links against; its presence means the target has the GTK stack.
pub fn target_has_gtk(target: &Path) -> bool {
    ["usr/lib64/libadwaita-1.so.0", "usr/lib/libadwaita-1.so.0"]
        .iter()
        .any(|p| target.join(p).exists())
}

/// `usr/bin/ustan` → `<target>/usr/local/bin/ustan`.
fn destination(target: &Path, rel: &str) -> PathBuf {
    target
        .join("usr/local")
        .join(rel.strip_prefix("usr/").unwrap_or(rel))
}

/// Copies ustan from `live_root` into `target`. Returns what was installed (paths relative to `/usr/local`), empty when the live
/// system has no ustan.
pub async fn install_from_live(live_root: &Path, target: &Path) -> crate::Result<Vec<String>> {
    if !available(live_root) {
        return Ok(Vec::new());
    }
    let mut wanted = vec![CLI];
    if target_has_gtk(target) {
        wanted.push(GUI);
        wanted.extend(GUI_FILES.iter().copied());
    }
    let mut done = Vec::new();
    for rel in wanted {
        let from = live_root.join(rel);
        if !from.is_file() {
            continue;
        }
        let to = destination(target, rel);
        if let Some(dir) = to.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        tokio::fs::copy(&from, &to).await?; // keeps the permission bits
        done.push(
            to.strip_prefix(target.join("usr/local"))
                .unwrap_or(&to)
                .display()
                .to_string(),
        );
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "gentoo-installer-ustan-{tag}-{}",
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

    fn live_with_ustan(tag: &str) -> PathBuf {
        let live = tree(tag);
        put(&live, CLI, 0o755);
        put(&live, GUI, 0o755);
        for f in GUI_FILES {
            put(&live, f, 0o644);
        }
        live
    }

    #[tokio::test]
    async fn a_target_without_gtk_gets_only_the_command_line_tool() {
        let (live, target) = (live_with_ustan("live1"), tree("target1"));
        let done = install_from_live(&live, &target).await.unwrap();
        assert_eq!(done, ["bin/ustan"]);
        assert!(installed(&target));
        assert!(!target.join("usr/local/bin/ustan-gui").exists());
        assert!(!target.join("usr/local/share/applications").exists());
    }

    #[tokio::test]
    async fn a_target_with_libadwaita_gets_the_window_and_its_menu_entry_too() {
        let (live, target) = (live_with_ustan("live2"), tree("target2"));
        put(&target, "usr/lib64/libadwaita-1.so.0", 0o755);
        let done = install_from_live(&live, &target).await.unwrap();
        assert_eq!(done.len(), 5, "{done:?}");
        for f in [
            "bin/ustan",
            "bin/ustan-gui",
            "share/applications/io.github.tarilka0gg.Ustan.desktop",
            "share/icons/hicolor/scalable/apps/ustan.svg",
        ] {
            assert!(target.join("usr/local").join(f).is_file(), "{f}");
        }
    }

    #[tokio::test]
    async fn the_copy_stays_executable_and_a_second_run_is_harmless() {
        use std::os::unix::fs::PermissionsExt;
        let (live, target) = (live_with_ustan("live3"), tree("target3"));
        install_from_live(&live, &target).await.unwrap();
        install_from_live(&live, &target).await.unwrap();
        let mode = std::fs::metadata(target.join("usr/local/bin/ustan"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }

    #[tokio::test]
    async fn a_live_system_without_ustan_installs_nothing() {
        let (live, target) = (tree("live4"), tree("target4"));
        assert!(!available(&live));
        assert!(install_from_live(&live, &target).await.unwrap().is_empty());
        assert!(!installed(&target));
    }
}
