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
            layouts.push(Layout {
                code: code.to_string(),
                description: description.trim().to_string(),
            });
        }
    }
    layouts
}

/// Best-effort current layout of the live console. OpenRC keeps it in `/etc/conf.d/keymaps`
/// (`keymap="us"`); `/etc/vconsole.conf` (`KEYMAP=`) is systemd's file and is only a fallback for
/// a live environment that has it. Anything else is `"us"`.
pub fn detect_current() -> String {
    detect_from(
        Path::new("/etc/conf.d/keymaps"),
        Path::new("/etc/vconsole.conf"),
    )
}

fn detect_from(openrc: &Path, vconsole: &Path) -> String {
    let value = |path: &Path, key: &str| {
        std::fs::read_to_string(path).ok().and_then(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.starts_with('#'))
                .find_map(|l| {
                    l.strip_prefix(key)
                        .map(|v| v.trim().trim_matches('"').to_string())
                })
        })
    };
    value(openrc, "keymap=")
        .or_else(|| value(vconsole, "KEYMAP="))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "us".to_string())
}

/// The layout a console uses: the first of a comma-separated list (`us,ua` -> `us`).
pub fn primary(layouts: &str) -> &str {
    layouts
        .split(',')
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("us")
}

/// Writes the Gentoo/OpenRC keymap config (`/etc/conf.d/keymaps`, `keymap="xx"`) into
/// the target, for the console: the first layout of the list. No chroot needed — this is a plain text file.
pub async fn apply(target: &Path, layouts: &str) -> crate::Result<()> {
    let conf_d = target.join("etc/conf.d");
    tokio::fs::create_dir_all(&conf_d).await?;
    let contents = format!("keymap=\"{}\"\n", primary(layouts));
    tokio::fs::write(conf_d.join("keymaps"), contents).await?;
    Ok(())
}

/// Rewrites the `xkb { ... }` block of a niri config so the layouts and the key that switches them are explicit
/// (an empty `xkb {}` means the US layout on a system without systemd). `None` when the config has no such block.
pub fn patch_niri_xkb(kdl: &str, layouts: &str, options: &str) -> Option<String> {
    let lines: Vec<&str> = kdl.lines().collect();
    let start = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with("xkb") && t[3..].trim_start().starts_with('{')
    })?;
    let indent: String = lines[start]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let end = if lines[start].contains('}') {
        start
    } else {
        start + lines[start..].iter().position(|l| l.trim() == "}")?
    };
    let mut block = vec![format!("{indent}xkb {{")];
    block.push(format!("{indent}    layout \"{layouts}\""));
    if !options.is_empty() {
        block.push(format!("{indent}    options \"{options}\""));
    }
    block.push(format!("{indent}}}"));
    let mut out: Vec<String> = lines[..start].iter().map(|l| l.to_string()).collect();
    out.extend(block);
    out.extend(lines[end + 1..].iter().map(|l| l.to_string()));
    Some(out.join("\n") + "\n")
}

/// The graphical session's layouts: niri's config of `username` and the X11 keyboard file (for Xwayland clients and
/// other compositors that read it). Does nothing for the plain US layout without a switch key.
pub async fn apply_session(
    target: &Path,
    username: &str,
    layouts: &str,
    options: &str,
) -> crate::Result<()> {
    if layouts.trim().is_empty() || (layouts.trim() == "us" && options.is_empty()) {
        return Ok(());
    }
    let xorg = target.join("etc/X11/xorg.conf.d");
    tokio::fs::create_dir_all(&xorg).await?;
    let mut conf = format!(
        "Section \"InputClass\"\n    Identifier \"keyboard layouts\"\n    MatchIsKeyboard \"on\"\n    Option \"XkbLayout\" \"{layouts}\"\n"
    );
    if !options.is_empty() {
        conf.push_str(&format!("    Option \"XkbOptions\" \"{options}\"\n"));
    }
    conf.push_str("EndSection\n");
    tokio::fs::write(xorg.join("00-keyboard.conf"), conf).await?;
    let path = target.join(format!("home/{username}/.config/niri/config.kdl"));
    if let Ok(kdl) = tokio::fs::read_to_string(&path).await {
        if let Some(patched) = patch_niri_xkb(&kdl, layouts, options) {
            tokio::fs::write(&path, patched).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_layouts_from_this_machine() {
        let layouts = list_layouts();
        assert!(
            !layouts.is_empty(),
            "expected {XKB_BASE_LIST} to be readable on this dev machine"
        );
        assert!(layouts.iter().any(|l| l.code == "us"));
    }

    #[test]
    fn the_console_takes_the_first_layout_of_the_list() {
        assert_eq!(primary("ua,us"), "ua");
        assert_eq!(primary(" us "), "us");
        assert_eq!(primary(""), "us");
    }

    #[test]
    fn the_xkb_block_of_a_niri_config_gets_the_layouts_and_the_switch_key() {
        let kdl = "input {\n    keyboard {\n        xkb {}\n        repeat-delay 250\n    }\n}\n";
        let out = patch_niri_xkb(kdl, "us,ua", "grp:alt_shift_toggle").unwrap();
        assert!(out.contains("        xkb {\n            layout \"us,ua\"\n            options \"grp:alt_shift_toggle\"\n        }\n        repeat-delay 250"));
        let again = patch_niri_xkb(&out, "de", "").unwrap();
        assert!(again.contains("layout \"de\"") && !again.contains("options"));
        assert!(!again.contains("us,ua"));
        assert_eq!(patch_niri_xkb("input {}\n", "us", ""), None);
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
        assert_eq!(
            detect_from(&o, &v),
            "de",
            "systemd's file is only a fallback"
        );
        std::fs::remove_file(&v).unwrap();
        assert_eq!(detect_from(&o, &v), "us");
        std::fs::write(&o, "keymap=\"\"\n").unwrap();
        assert_eq!(detect_from(&o, &v), "us", "an empty value is no value");
        std::fs::remove_dir_all(&dir).ok();
    }
}
