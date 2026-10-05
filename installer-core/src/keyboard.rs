//! Keyboard layout: detection + list, for the installer's optional "Advanced setup"
//! manual override (auto-detected by default).

use std::path::Path;

const XKB_BASE_LIST: &str = "/usr/share/X11/xkb/rules/base.lst";

#[derive(Debug, Clone)]
pub struct Layout {
    pub code: String,
    pub description: String,
}

/// Parses the `! layout` section of the system's XKB rules list — the same source
/// `setxkbmap`/`localectl` draw from, so the codes line up with what the installed
/// system's console/X keymap actually expects.
pub fn list_layouts() -> Vec<Layout> {
    let Ok(text) = std::fs::read_to_string(XKB_BASE_LIST) else {
        return Vec::new();
    };

    let mut layouts = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        if line.starts_with('!') {
            in_section = line.trim() == "! layout";
            continue;
        }
        if !in_section {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((code, description)) = trimmed.split_once(char::is_whitespace) {
            layouts.push(Layout { code: code.to_string(), description: description.trim().to_string() });
        }
    }
    layouts
}

/// Best-effort current layout of the live console. OpenRC keeps it in `/etc/conf.d/keymaps`
/// (`keymap="us"`); `/etc/vconsole.conf` (`KEYMAP=`) is systemd's file and is only a fallback for
/// a live environment that has it. Anything else is `"us"`.
pub fn detect_current() -> String {
    detect_from(Path::new("/etc/conf.d/keymaps"), Path::new("/etc/vconsole.conf"))
}

fn detect_from(openrc: &Path, vconsole: &Path) -> String {
    let value = |path: &Path, key: &str| {
        std::fs::read_to_string(path).ok().and_then(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.starts_with('#'))
                .find_map(|l| l.strip_prefix(key).map(|v| v.trim().trim_matches('"').to_string()))
        })
    };
    value(openrc, "keymap=")
        .or_else(|| value(vconsole, "KEYMAP="))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "us".to_string())
}

/// Writes the Gentoo/OpenRC keymap config (`/etc/conf.d/keymaps`, `keymap="xx"`) into
/// the target. No chroot needed — this is a plain text file.
pub async fn apply(target: &Path, layout: &str) -> crate::Result<()> {
    let conf_d = target.join("etc/conf.d");
    tokio::fs::create_dir_all(&conf_d).await?;
    let contents = format!("keymap=\"{layout}\"\n");
    tokio::fs::write(conf_d.join("keymaps"), contents).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_layouts_from_this_machine() {
        let layouts = list_layouts();
        assert!(!layouts.is_empty(), "expected {XKB_BASE_LIST} to be readable on this dev machine");
        assert!(layouts.iter().any(|l| l.code == "us"));
    }

    #[test]
    fn detect_current_never_panics_and_has_a_fallback() {
        let current = detect_current();
        assert!(!current.is_empty());
    }

    #[test]
    fn the_openrc_keymap_file_wins_and_comments_are_ignored() {
        let dir = std::env::temp_dir().join(format!("gi-kbd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (o, v) = (dir.join("keymaps"), dir.join("vconsole.conf"));
        std::fs::write(&o, "# keymap=\"xx\"\nkeymap=\"ua\"\nwindowkeys=\"YES\"\n").unwrap();
        std::fs::write(&v, "KEYMAP=de\n").unwrap();
        assert_eq!(detect_from(&o, &v), "ua");
        std::fs::remove_file(&o).unwrap();
        assert_eq!(detect_from(&o, &v), "de", "systemd's file is only a fallback");
        std::fs::remove_file(&v).unwrap();
        assert_eq!(detect_from(&o, &v), "us");
        std::fs::write(&o, "keymap=\"\"\n").unwrap();
        assert_eq!(detect_from(&o, &v), "us", "an empty value is no value");
        std::fs::remove_dir_all(&dir).ok();
    }
}
