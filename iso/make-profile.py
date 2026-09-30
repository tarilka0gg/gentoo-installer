#!/usr/bin/env python3
"""make-profile.py <out-dir> — turn the current user's niri/Noctalia setup into a live-ISO profile.

Reads ~/.config/niri/config.kdl, ~/.config/noctalia/config.toml and
~/.local/state/noctalia/{settings,state}.toml, and writes a copy that is safe to boot on
someone else's hardware:

  kept     look and layout (colours, gaps, animations, key bindings, input), bar + dock layout,
           theme and the community palettes/templates it refers to
  dropped  everything tied to this machine or person: start-up services and scripts, monitor
           (output) blocks, the iGPU render-device pin, per-app window rules, the home address,
           wallpaper paths, per-monitor lock-screen widgets, history/usage/clipboard data

The output is NOT meant for git: it is derived from a personal home directory.
"""
import os, re, shutil, sys
from pathlib import Path

HOME = Path.home()
out = Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)


def split_blocks(text):
    """Yield (kind, chunk): top-level `name ... { ... }` blocks (brace-matched) and other lines."""
    lines, i, res = text.splitlines(keepends=True), 0, []
    while i < len(lines):
        line = lines[i]
        code = line.split("//")[0]
        if code.count("{") > code.count("}") and not line.startswith((" ", "\t")):
            depth, chunk = 0, []
            while i < len(lines):
                c = lines[i].split("//")[0]
                depth += c.count("{") - c.count("}")
                chunk.append(lines[i]); i += 1
                if depth <= 0:
                    break
            res.append(("block", "".join(chunk)))
        else:
            res.append(("line", line)); i += 1
    return res


DROP_RULE = re.compile(r'omniroute|app-id="zen"|telegram|ElyPrism|Minecraft|btop-monitor')
DROP_LINE = re.compile(
    r'^\s*(spawn-at-startup|spawn-sh-at-startup)\b(?!.*(xwayland-satellite|"noctalia"))'  # keep the two we need
    r'|render-drm-device|xcursor-theme|screenshot-path'
    r'|iGPU|RTX|XDG_MENU_PREFIX|INIR_VENV|ILLOGICAL_IMPULSE|QT_QPA_PLATFORMTHEME|EDITOR "'
    r'|"zen"|copy-latest-screenshot|"yazi"|"nvim"|xdg-open'
)


def niri(src):
    keep = []
    for kind, chunk in split_blocks(src):
        if kind == "block":
            head = chunk.lstrip().split("{")[0].strip()
            if head.startswith("output "):
                continue                                   # this machine's monitors
            if head == "window-rule" and DROP_RULE.search(chunk):
                continue                                   # apps the ISO does not have
            chunk = "".join(l for l in chunk.splitlines(keepends=True) if not DROP_LINE.search(l))
            keep.append(chunk)
        elif not DROP_LINE.search(chunk):
            keep.append(chunk)
    text = "".join(keep)
    text = text.replace('debug {\n', 'debug {\n', 1)
    text += '\nscreenshot-path "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png"\n'
    return text


KEEP_SECTIONS = {"bar", "control_center", "desktop_widgets", "dock", "hot_corners", "idle",
                 "lockscreen", "nightlight", "osd", "shell", "theme", "widget", "wallpaper_panel"}


def settings(src):
    out_lines, keep = [], True
    for line in src.splitlines(keepends=True):
        m = re.match(r"\s*\[\[?([A-Za-z0-9_.\"@-]+)", line)
        if m and not line.lstrip().startswith("#"):
            keep = m.group(1).split(".")[0] in KEEP_SECTIONS
        if not keep:
            continue
        if re.match(r"\s*font_family\s*=", line):
            continue                                        # the font is not on the ISO
        if re.match(r"\s*pinned\s*=", line):
            line = re.sub(r"\[.*\]", '[ "com.mitchellh.ghostty" ]', line)   # only ghostty exists there
        out_lines.append(line)
    return "".join(out_lines)


(out / "niri").mkdir(exist_ok=True)
(out / "niri/config.kdl").write_text(niri((HOME / ".config/niri/config.kdl").read_text()))

(out / "noctalia").mkdir(exist_ok=True)
cfg = (HOME / ".config/noctalia/config.toml").read_text()
cfg = re.sub(r'(?m)^(directory\s*=\s*)".*Wallpapers"', r'\1"~/Pictures/Wallpapers"', cfg)
(out / "noctalia/config.toml").write_text(cfg)

state = HOME / ".local/state/noctalia"
dst = out / "state/noctalia"; dst.mkdir(parents=True, exist_ok=True)
(dst / "settings.toml").write_text(settings((state / "settings.toml").read_text()))
shutil.copy(state / "state.toml", dst / "state.toml")
for d in ("community-palettes", "community-templates"):    # public downloads, not personal data
    if (state / d).is_dir():
        shutil.copytree(state / d, dst / d, dirs_exist_ok=True)
print("wrote", out)
