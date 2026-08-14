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

/// Best-effort current layout: the live console keymap if it maps cleanly to an XKB
/// layout code (true for the common case — `us`, `ua`, `de`, ...), else `"us"`.
pub fn detect_current() -> String {
    std::fs::read_to_string("/etc/vconsole.conf")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("KEYMAP=").map(|v| v.trim_matches('"').to_string()))
        })
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
}
