//! Time zone: detection + list, for the installer's optional "Advanced setup" manual
//! override (auto-detected by default — same detection `detect::gather` already does).

use std::path::Path;

const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";

/// `Region/City` zone names, walked straight off the live medium's own zoneinfo data
/// rather than a bundled list — it's already correct and already there. Skips the
/// `posix/`/`right/` alias trees and non-zone files (`posixrules`, `Factory`, the
/// top-level `*.tab` metadata files) so the list is just real, selectable zones.
pub fn list_zones() -> Vec<String> {
    let mut zones = Vec::new();
    walk(Path::new(ZONEINFO_DIR), "", &mut zones);
    zones.sort();
    zones
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    const SKIP_DIRS: &[&str] = &["posix", "right"];
    const SKIP_FILES: &[&str] = &["posixrules", "Factory", "localtime"];

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('+') || name.ends_with(".tab") || name.ends_with(".list") {
            continue;
        }
        let path = entry.path();
        let qualified = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };

        if path.is_dir() {
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            walk(&path, &qualified, out);
        } else if !SKIP_FILES.contains(&name) {
            out.push(qualified);
        }
    }
}

/// Live-environment detection: `/etc/timezone` if present, else the `/etc/localtime`
/// symlink target. `None` if neither resolves — caller defaults to UTC.
pub fn detect_current() -> Option<String> {
    std::fs::read_to_string("/etc/timezone")
        .ok()
        .map(|s| s.trim().to_string())
        .or_else(|| {
            std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                p.strip_prefix("/usr/share/zoneinfo/")
                    .ok()
                    .map(|p| p.display().to_string())
            })
        })
}

/// Writes `/etc/timezone` and symlinks `/etc/localtime` in the target — the two files
/// Gentoo's `sys-libs/timezone-data` expects, both plain filesystem operations that need
/// no chroot.
pub async fn apply(target: &Path, zone: &str) -> crate::Result<()> {
    tokio::fs::create_dir_all(target.join("etc")).await?;
    tokio::fs::write(target.join("etc/timezone"), format!("{zone}\n")).await?;

    let localtime = target.join("etc/localtime");
    tokio::fs::remove_file(&localtime).await.ok();
    let relative = format!("../usr/share/zoneinfo/{zone}");
    tokio::fs::symlink(&relative, &localtime).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_known_zones_from_this_machine() {
        let zones = list_zones();
        assert!(
            !zones.is_empty(),
            "expected {ZONEINFO_DIR} to be readable on this dev machine"
        );
        assert!(zones.iter().any(|z| z == "Europe/Kyiv"));
    }

    #[test]
    fn skips_alias_trees_and_metadata_files() {
        let zones = list_zones();
        assert!(!zones
            .iter()
            .any(|z| z.starts_with("posix/") || z.starts_with("right/")));
        assert!(!zones.iter().any(|z| z == "zone.tab" || z == "iso3166.tab"));
    }
}
