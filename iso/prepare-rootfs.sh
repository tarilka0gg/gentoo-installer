#!/bin/bash
# prepare-rootfs.sh <rootfs> <installer-cli-binary> [installer-gui-binary simple-linux-configs-dir [profile-dir]]
# Turns an unpacked stage3 (with the runtime tools already emerged) into the live system:
# root autologin on tty1 + serial, the installer started from root's login shell, iwd and
# dhcpcd on boot, an empty root password (this is a live ISO, not an installed system).
set -euo pipefail
ROOT=${1:?rootfs}
CLI=${2:?installer-cli binary}
GUI=${3:-}
WMCONF=${4:-}
PROFILE=${5:-}     # optional output of make-profile.py: the user's own niri/Noctalia setup

install -Dm755 "$CLI" "$ROOT/usr/local/bin/installer-cli"
"$(dirname "$(readlink -f "$0")")/strip-isa-note.sh" "$ROOT/usr/local/bin/installer-cli"

# Empty root password + autologin on both consoles. The serial line is what QEMU tests use.
chroot "$ROOT" passwd -d root >/dev/null
sed -i -E '/^c1:/d; /^s0:/d' "$ROOT/etc/inittab"
cat >> "$ROOT/etc/inittab" <<'INITTAB'
c1:12345:respawn:/sbin/agetty --autologin root --noclear 38400 tty1 linux
s0:12345:respawn:/usr/local/sbin/serial-getty
INITTAB
# A serial login only where the kernel was told to use one (`console=ttyS0`); otherwise an agetty
# on a port that may not exist respawns in a loop and spams the console. Idle instead.
install -d "$ROOT/usr/local/sbin"
cat > "$ROOT/usr/local/sbin/serial-getty" <<'SERIALGETTY'
#!/bin/sh
if grep -q 'console=ttyS0' /proc/cmdline; then
    exec /sbin/agetty --autologin root -L 115200 ttyS0 vt100
fi
exec sleep 2147483647
SERIALGETTY
chmod 755 "$ROOT/usr/local/sbin/serial-getty"
grep -q '^ttyS0$' "$ROOT/etc/securetty" 2>/dev/null || echo ttyS0 >> "$ROOT/etc/securetty"

# Start the installer on the first console that logs in; leaving it drops to a shell.
# With a GUI binary, tty1 starts the user's own niri + Noctalia session (config from the
# simple-linux-configs repo) with installer-gui launched on top, full screen; if the session
# ends, the text installer takes over. The serial console always gets the TUI.
if [ -n "$GUI" ]; then
    install -Dm755 "$GUI" "$ROOT/usr/local/bin/installer-gui"
"$(dirname "$(readlink -f "$0")")/strip-isa-note.sh" "$ROOT/usr/local/bin/installer-gui"

    # Zen Browser is not on the medium (it was 113 MB compressed): the graphical session fetches it in the background from the project's
    # own GitHub release as soon as it starts (`get-zen --fetch`); starting Zen earlier just waits for that download. No terminal.
    cat > "$ROOT/usr/local/bin/get-zen" <<'GETZEN'
#!/bin/sh
# get-zen [--fetch] [args]   Starts Zen Browser, downloading it first if this live session has not got it yet. It is fetched over HTTPS
# from the project's GitHub release (no checksum is published there) into ~/.local/share/zen, which lives in RAM on the live system.
#   --fetch  only download (quietly, waiting for the network if needed); used at session start so Zen is ready when you open it.
# One download at a time: a second caller waits for the first one's lock.
set -eu
D=${XDG_DATA_HOME:-$HOME/.local/share}/zen
BIN=$D/zen/zen-bin
URL=${ZEN_URL:-https://github.com/zen-browser/desktop/releases/latest/download/zen.linux-x86_64.tar.xz}

fetch() {
    mkdir -p "$D"
    exec 9>"$D/.lock"; flock 9
    [ -x "$BIN" ] && return 0
    q=-s; [ -t 2 ] && q=--progress-bar
    n=0
    # No network yet right after boot: keep trying (about 10 minutes), the live session may still be bringing the link up.
    until curl -fL --retry 5 --retry-delay 3 --retry-all-errors $q -o "$D/zen.tar.xz" "$URL"; do
        n=$((n + 1)); [ "$n" -ge 120 ] && { echo "get-zen: download failed" >&2; return 1; }
        sleep 5
    done
    tar -C "$D" -xf "$D/zen.tar.xz"
    rm -f "$D/zen.tar.xz"
    [ -x "$BIN" ] || { echo "get-zen: $BIN missing after unpacking" >&2; return 1; }
}

if [ "${1:-}" = --fetch ]; then
    [ -x "$BIN" ] || fetch
    exit 0
fi
if [ ! -x "$BIN" ]; then
    command -v notify-send >/dev/null 2>&1 && notify-send -a "Zen Browser" "Zen Browser is still downloading" "It will open as soon as the download finishes." || true
    fetch
fi
exec "$BIN" "$@"
GETZEN
    chmod 755 "$ROOT/usr/local/bin/get-zen"
    cat > "$ROOT/usr/share/applications/zen-browser.desktop" <<'ZENDESKTOP'
[Desktop Entry]
Type=Application
Name=Zen Browser
Comment=Zen Browser (downloaded in the background when the live session starts)
Icon=zen
Exec=get-zen %u
Terminal=false
Categories=Network;WebBrowser;
MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;
StartupWMClass=zen
ZENDESKTOP
    [ -n "$WMCONF" ] || { echo "GUI mode needs the simple-linux-configs directory" >&2; exit 1; }
    install -d "$ROOT/root/.config/niri" "$ROOT/root/.config/noctalia"
    cp "$WMCONF/niri/config.kdl" "$ROOT/root/.config/niri/config.kdl"
    cp "$WMCONF/noctalia/config.toml" "$ROOT/root/.config/noctalia/config.toml"
    if [ -n "$PROFILE" ]; then
        # The user's own look, bar and dock (already stripped of machine-specific parts by
        # make-profile.py) replace the repo presets.
        cp "$PROFILE/niri/config.kdl" "$ROOT/root/.config/niri/config.kdl"
        cp "$PROFILE/noctalia/config.toml" "$ROOT/root/.config/noctalia/config.toml"
        install -d "$ROOT/root/.local/state"
        cp -r "$PROFILE/state/noctalia" "$ROOT/root/.local/state/"
    else
        # No personal profile: the repo's presets above are the whole look. A tree that an earlier build gave a profile keeps its
        # Noctalia state (wallpaper, theme) otherwise, and the image would show someone's own desktop.
        rm -rf "$ROOT/root/.local/state/noctalia"
    fi
    # The live image's look, whoever's bar and settings the config above came from: colours generated from the wallpaper
    # (Material You, m3-tonal-spot, light), the wallpapers of this project and no one's own wallpaper folder. A personal profile
    # normally pins a palette ("community"/"builtin") and its own wallpaper directory; neither belongs in the image.
    # The same two files twice: config.toml, and the saved settings of a personal profile (state/noctalia/settings.toml: the bar
    # layout, widgets, and also a [theme] that takes precedence over config.toml).
    for noctalia_file in "$ROOT/root/.config/noctalia/config.toml" "$ROOT/root/.local/state/noctalia/settings.toml"; do
    [ -f "$noctalia_file" ] || continue
    python3 - "$noctalia_file" <<'THEMEPY'
import re, sys
path = sys.argv[1]
text = open(path).read()
settings_file = path.endswith('settings.toml')
def section(name, fn):
    global text
    m = re.search(r'(?ms)^\[%s\]\n(.*?)(?=^\[|\Z)' % re.escape(name), text)
    if not m:
        text += '\n[%s]\n' % name; m = re.search(r'(?ms)^\[%s\]\n(.*?)(?=^\[|\Z)' % re.escape(name), text)
    text = text[:m.start(1)] + fn(m.group(1)) + text[m.end(1):]
def setkey(body, key, value):
    line = '%s = "%s"' % (key, value)
    if re.search(r'(?m)^%s\s*=' % key, body):
        return re.sub(r'(?m)^%s\s*=[^\n]*' % key, line, body, count=1)
    if re.search(r'(?m)^#\s*%s\s*=' % key, body):
        return re.sub(r'(?m)^#\s*%s\s*=[^\n]*' % key, line, body, count=1)
    return line + '\n' + body
def theme(body):
    body = setkey(body, 'mode', 'light')
    body = setkey(body, 'source', 'wallpaper')
    body = setkey(body, 'wallpaper_scheme', 'm3-tonal-spot')
    # the author's own palette is not part of the image
    return re.sub(r'(?m)^community_palette\s*=[^\n]*\n', '', body)
section('theme', theme)
# Icons recoloured to a palette role: a profile tuned for a dark palette says "surface", which on a light wallpaper palette is the
# colour of the dock itself and the icons vanish. The role of the documented default is readable on any palette.
if settings_file:
    section('shell', lambda b: re.sub(r'(?m)^(app_icon_color\s*=\s*)"[^"]*"', r'\1"on_surface"', b))
if not settings_file:
    section('wallpaper', lambda b: setkey(b, 'directory', '~/Pictures/Wallpapers'))
open(path, 'w').write(text)
THEMEPY
    done
    rm -f "$ROOT"/root/.local/state/noctalia/community-palettes/Futuro*
    # GTK look: $THEME_ASSETS holds icons/<theme> and themes/<theme> directories (WhiteSur icons;
    # optional GTK themes) that are copied in as they are; the settings point GTK 3/4 at them.
        if [ -n "${THEME_ASSETS:-}" ] && [ -d "$THEME_ASSETS" ]; then
        for kind in icons themes; do
            [ -d "$THEME_ASSETS/$kind" ] || continue
            install -d "$ROOT/usr/share/$kind"
            for t in "$THEME_ASSETS/$kind"/*; do
                rm -rf "$ROOT/usr/share/$kind/$(basename "$t")"
                cp -a "$t" "$ROOT/usr/share/$kind/"
            done
        done
    fi
    for v in 3.0 4.0; do
        install -d "$ROOT/root/.config/gtk-$v"
        printf '[Settings]\ngtk-theme-name=Adwaita\ngtk-icon-theme-name=WhiteSur-dark\ngtk-application-prefer-dark-theme=false\n' \
            > "$ROOT/root/.config/gtk-$v/settings.ini"
        # Noctalia's gtk3/gtk4 templates write noctalia.css (colours from the wallpaper) here.
        printf '@import url("noctalia.css");\n' > "$ROOT/root/.config/gtk-$v/gtk.css"
    done
    # A different wallpaper on every start: wait for Noctalia to listen, then ask for a random one
    # (it picks from ~/Pictures/Wallpapers, sub-folders included).
    cat > "$ROOT/usr/local/bin/random-wallpaper" <<'RANDWP'
#!/bin/sh
# Noctalia answers on its socket a moment before it applies wallpapers, so ask until the
# wallpaper it reports is a real file.
for _ in $(seq 1 60); do
    noctalia msg wallpaper-random >/dev/null 2>&1
    sleep 2
    wp=$(noctalia msg wallpaper-get 2>/dev/null | head -n1)
    [ -n "$wp" ] && [ -f "$wp" ] && exit 0
done
RANDWP
    chmod 755 "$ROOT/usr/local/bin/random-wallpaper"
    # A launcher entry for the installer: without it the dock/launcher have no icon or name for
    # its window (app id org.gentoo_diy.Installer).
    cat > "$ROOT/usr/share/applications/org.gentoo_diy.Installer.desktop" <<'INSTDESKTOP'
[Desktop Entry]
Type=Application
Name=Install Simple Linux
Comment=Install Gentoo with btrfs, a prebuilt kernel and Limine
Exec=installer-gui
Icon=system-software-install
Categories=System;
StartupWMClass=org.gentoo_diy.Installer
INSTDESKTOP
    # Noctalia's niri template fills noctalia.kdl (focus ring, borders) from the wallpaper palette;
    # niri has to find the file at start, so an empty one is there until Noctalia writes it.
    : > "$ROOT/root/.config/niri/noctalia.kdl"
    grep -q '^include "noctalia.kdl"' "$ROOT/root/.config/niri/config.kdl" \
        || printf '\ninclude "noctalia.kdl"\n' >> "$ROOT/root/.config/niri/config.kdl"
    # Launcher entries nobody needs in a live session (console tools included; the zen-bin package ships its own "Zen"
    # entry next to ours, which is the one that downloads the browser).
    for d in zen-zen-bin dev.noctalia.Noctalia panel-preferences xfce4-about thunar-bulk-rename thunar-settings btop micro; do
        f="$ROOT/usr/share/applications/$d.desktop"
        [ -f "$f" ] && ! grep -q '^NoDisplay=true' "$f" && sed -i '0,/^\[Desktop Entry\]/s//[Desktop Entry]\nNoDisplay=true/' "$f"
    done
    # Noctalia recolours application icons to the wallpaper palette, which only gives a readable
    # picture for glyph-style (symbolic) icons; the full-colour ones turn into solid blobs. So the
    # launcher entries point at symbolic glyphs.
    sym() { f="$ROOT/usr/share/applications/$1.desktop"; [ -f "$f" ] && sed -i "0,/^Icon=.*/s//Icon=$2/" "$f"; return 0; }
    sym com.mitchellh.ghostty utilities-terminal-symbolic
    sym gparted drive-harddisk-symbolic
    sym org.gentoo_diy.Installer system-software-install-symbolic
    sym io.github.tarilka0gg.PortageStore package-x-generic-symbolic
    sym thunar system-file-manager-symbolic
    sym io.github.tarilka0gg.Ustan application-x-executable-symbolic
    sym zen-browser web-browser-symbolic
    # GTK 4 under Wayland takes the icon/GTK theme from GSettings before settings.ini, so the
    # same names go in as schema defaults.
    install -d "$ROOT/usr/share/glib-2.0/schemas"
    printf "[org.gnome.desktop.interface]\nicon-theme='WhiteSur-dark'\ngtk-theme='Adwaita'\ncolor-scheme='default'\n" \
        > "$ROOT/usr/share/glib-2.0/schemas/90_simple-linux.gschema.override"
    chroot "$ROOT" glib-compile-schemas /usr/share/glib-2.0/schemas
    # Wallpapers: Noctalia's wallpaper directory is ~/Pictures/Wallpapers, so the Simple Linux
    # set lands in its "simple" sub-folder. $WALLPAPERS = a directory holding that set.
    if [ -n "${WALLPAPERS:-}" ] && [ -d "$WALLPAPERS" ]; then
        rm -rf "$ROOT/root/Pictures/Wallpapers/simple"
        install -d "$ROOT/root/Pictures/Wallpapers/simple"
        cp -r "$WALLPAPERS"/. "$ROOT/root/Pictures/Wallpapers/simple/"
    fi
    # Live-only additions, appended to the *copy*: start the installer and give it the whole
    # screen. The preset itself is left exactly as the repo has it.
    # Noctalia shows a first-run wizard on top of everything until this marker exists; on a
    # live ISO it would cover the installer. The marker sits next to state.toml.
    install -d "$ROOT/root/.local/state/noctalia"
    touch "$ROOT/root/.local/state/noctalia/.setup-complete"
    # gnome-keyring asks for a password to create "Default keyring" the first time Noctalia
    # touches the secret service, and that prompt lands on top of the installer. A plain
    # (unencrypted, empty-password) default keyring makes the question moot; nothing secret
    # lives in a live session.
    install -d -m 700 "$ROOT/root/.local/share/keyrings"
    printf 'Default_keyring' > "$ROOT/root/.local/share/keyrings/default"
    cat > "$ROOT/root/.local/share/keyrings/Default_keyring.keyring" <<'KEYRING'
[keyring]
display-name=Default keyring
ctime=0
mtime=0
lock-on-idle=false
lock-after=false
KEYRING
    chmod 600 "$ROOT/root/.local/share/keyrings/Default_keyring.keyring"
    cat >> "$ROOT/root/.config/niri/config.kdl" <<'NIRILIVE'

// --- live ISO additions ---
spawn-at-startup "pipewire"
spawn-at-startup "wireplumber"
spawn-at-startup "pipewire-pulse"
spawn-at-startup "installer-gui"
spawn-at-startup "get-zen" "--fetch"
spawn-at-startup "random-wallpaper"
window-rule {
    match app-id="org.gentoo_diy.Installer"
    open-floating true
    default-column-width { fixed 800; }
    default-window-height { fixed 640; }   // 5:4
}
NIRILIVE
    install -d "$ROOT/usr/local/bin"
    cat > "$ROOT/usr/local/bin/live-gui" <<'LIVEGUI'
#!/bin/sh
# live-gui: the graphical live session with two fallbacks; the caller starts the text installer afterwards.
#   1. niri on real graphics. niri refuses software rendering, so without a usable GPU it never gets an output.
#   2. cage (wlroots' pixman renderer) + GTK's cairo renderer: a graphical installer with no 3D at all.
# Everything is logged to /var/log/live-gui.log. Processes are stopped by PID, not by name.
LOG=/var/log/live-gui.log
log() { echo "$(date +%T) $*" >> "$LOG"; }
export XDG_RUNTIME_DIR=/run/user/0 LIBSEAT_BACKEND=seatd
mkdir -p -m 700 "$XDG_RUNTIME_DIR"

# Is there a GPU driver that can do 3D? simpledrm/efifb/bochs/cirrus/qxl cannot, and niri then sits there without an output
# (it used to be waited out for 25 s). The driver binds a second or two after udev starts: wait for udev, then up to 3 s more.
accel_gpu() {
    for c in /sys/class/drm/card[0-9]*; do
        [ -e "$c/device/driver" ] || continue
        case $(basename "$(readlink -f "$c/device/driver")") in
            i915|xe|amdgpu|radeon|nouveau|nvidia|virtio_gpu|vmwgfx|msm|panfrost|panthor|v3d|vc4|etnaviv|lima) return 0 ;;
        esac
    done
    ls /dev/dri/renderD* >/dev/null 2>&1
}
# GPU machines must not pay for this: look for a 3D driver for 2 s first. Only if there is none, let udev finish loading drivers
# (bounded) and look 1.5 s more; on a machine whose cards are all unaccelerated that settles the question in about 3 s.
t=0
while ! accel_gpu && [ "$t" -lt 4 ]; do sleep 0.5; t=$((t + 1)); done
if ! accel_gpu; then
    udevadm settle --timeout=5 >/dev/null 2>&1
    t=0
    while ! accel_gpu && [ "$t" -lt 3 ]; do sleep 0.5; t=$((t + 1)); done
fi
if ! accel_gpu; then
    log "no 3D-capable GPU driver after $((t / 2))s: skipping niri"
else
    log "3D-capable GPU after $((t / 2))s; starting niri"
    dbus-run-session -- niri --session 2>/var/log/niri-session.log &
    pid=$!
    # Poll for an output twice a second, up to 15 s (was a fixed 25 s sleep, also when niri was already up).
    i=0; ok=
    while [ "$i" -lt 30 ] && kill -0 "$pid" 2>/dev/null; do
        sock=$(ls /run/user/0 2>/dev/null | grep -m1 '^niri\..*\.sock$')
        if [ -n "$sock" ] && [ -n "$(NIRI_SOCKET="/run/user/0/$sock" niri msg outputs 2>/dev/null)" ]; then ok=1; break; fi
        sleep 0.5; i=$((i + 1))
    done
    if [ -n "$ok" ]; then
        log "niri has an output after $((i / 2))s; leaving the session running"
        wait "$pid"
        log "niri session ended (status $?)"
        exit 0
    fi
    if kill -0 "$pid" 2>/dev/null; then
        kids=$(pgrep -P "$pid" 2>/dev/null | tr '\n' ' ')
        log "niri has no output after $((i / 2))s; stopping dbus-run-session $pid and children [$kids]"
        kill -TERM $kids "$pid" >> "$LOG" 2>&1
        sleep 1
        kill -KILL $kids "$pid" >> "$LOG" 2>&1
        pkill -KILL -x installer-gui >> "$LOG" 2>&1
        wait "$pid" 2>/dev/null
        log "niri stopped; still alive: [$(pgrep -x niri | tr '\n' ' ')]"
    else
        wait "$pid"
        log "niri exited by itself within $((i / 2))s (status $?)"
    fi
fi

echo "No hardware-accelerated graphics (niri needs it). Trying a software-rendered window..."
log "starting cage with the pixman renderer"
env WLR_RENDERER=pixman WLR_NO_HARDWARE_CURSORS=1 GSK_RENDERER=cairo cage -s -- installer-gui 2>/var/log/cage-session.log
log "cage exited (status $?)"
echo "The graphical session ended; starting the text installer. Logs: /var/log/live-gui.log, niri-session.log, cage-session.log"
LIVEGUI
    chmod 755 "$ROOT/usr/local/bin/live-gui"
    cat > "$ROOT/root/.bash_profile" <<'PROFILE'
if [ -z "${INSTALLER_STARTED:-}" ]; then
    export INSTALLER_STARTED=1
    case "$(tty)" in
    /dev/tty1)
        export XDG_RUNTIME_DIR=/run/user/0 LIBSEAT_BACKEND=seatd
        live-gui
        installer-cli
        ;;
    /dev/ttyS0)
        # `live.debug` on the kernel command line leaves a plain shell here.
        grep -qw live.debug /proc/cmdline || installer-cli
        ;;
    esac
    echo "Installer exited. This is a live shell; run 'installer-cli' to start the text installer."
fi
PROFILE
    chroot "$ROOT" rc-update add seatd default >/dev/null 2>&1 || true
else
    cat > "$ROOT/root/.bash_profile" <<'PROFILE'
if [ -z "${INSTALLER_STARTED:-}" ] && { [ "$(tty)" = /dev/tty1 ] || { [ "$(tty)" = /dev/ttyS0 ] && ! grep -qw live.debug /proc/cmdline; }; }; then
    export INSTALLER_STARTED=1
    installer-cli
    echo "Installer exited. This is a live shell; run 'installer-cli' to start it again."
fi
PROFILE
fi

# --- fish as root's shell, with the house aliases (micro, eza, dust, gping) ---
# The aliases match the author's own fish config; they live in /etc/fish/conf.d so every
# user of the image gets them. Only meaningful if the tools were emerged into the rootfs.
if [ -x "$ROOT/usr/bin/fish" ]; then
    install -d "$ROOT/etc/fish/conf.d"
    cat > "$ROOT/etc/fish/conf.d/10-house.fish" <<'HOUSE'
set -gx EDITOR micro
set -gx VISUAL micro
set fish_greeting
if status is-interactive
    alias ls 'eza --icons --group-directories-first'
    alias ll 'eza -la --icons --group-directories-first --git'
    alias lt 'eza --tree --icons --level=2'
    alias nano micro
    alias du dust
    alias ping gping
end
HOUSE
    grep -qx /usr/bin/fish "$ROOT/etc/shells" || echo /usr/bin/fish >> "$ROOT/etc/shells"
    chroot "$ROOT" usermod -s /usr/bin/fish root

    # Same start-up logic as .bash_profile, in fish syntax (the login shell is now fish).
    install -d "$ROOT/root/.config/fish"
    if [ -n "$GUI" ]; then
        cat > "$ROOT/root/.config/fish/config.fish" <<'FISHCONF'
if status is-login; and not set -q INSTALLER_STARTED
    set -gx INSTALLER_STARTED 1
    switch (tty)
    case /dev/tty1
        live-gui
        installer-cli
    case /dev/ttyS0
        grep -qw live.debug /proc/cmdline; or installer-cli
    end
    echo "Installer exited. This is a live shell; run 'installer-cli' to start the text installer."
end
FISHCONF
    else
        cat > "$ROOT/root/.config/fish/config.fish" <<'FISHCONF'
if status is-login; and not set -q INSTALLER_STARTED
    set -gx INSTALLER_STARTED 1
    # `live.debug` on the kernel command line leaves a plain shell on the serial console.
    if test (tty) = /dev/tty1; or begin; test (tty) = /dev/ttyS0; and not grep -qw live.debug /proc/cmdline; end
        installer-cli
        echo "Installer exited. This is a live shell; run 'installer-cli' to start it again."
    end
end
FISHCONF
    fi
fi

# Without a stage3 on the medium the installer downloads one. The default is this project's own (fish, eza, micro, the house
# aliases) from the latest release; its digest is read from the `.sha512` file next to it. Both shells get it, only when
# nothing is set already and no stage3 is shipped on the medium (STAGE_TARBALL builds keep using theirs).
STAGE_URL=${STAGE3_DEFAULT_URL:-https://github.com/tarilka0gg/simple-linux/releases/latest/download/simple-linux-stage3-amd64-openrc.tar.xz}
CONFIGS_URL=${WM_CONFIGS_DEFAULT_URL:-https://github.com/tarilka0gg/simple-linux-configs.git}
# The store (kernel builds, Portage overlay) has no public address yet, so no default is invented: a build that has one passes
# STORE_BINHOST_DEFAULT_URL and STORE_OVERLAY_DEFAULT_URL (a test build points them at its own servers). Without them the installer
# says "store not configured" until the variables are set by hand.
STORE_BINHOST=${STORE_BINHOST_DEFAULT_URL:-}
STORE_OVERLAY=${STORE_OVERLAY_DEFAULT_URL:-}
install -d "$ROOT/etc/profile.d" "$ROOT/etc/fish/conf.d"
cat > "$ROOT/etc/profile.d/installer-stage.sh" <<SHENV
# Written by prepare-rootfs.sh: where the installer gets its stage3 when the medium carries none.
if [ -z "\${GENTOO_INSTALLER_STAGE3_URL:-}" ] && [ ! -d /run/initramfs/live/stage ] && [ ! -d /run/live/medium/stage ]; then
    export GENTOO_INSTALLER_STAGE3_URL=$STAGE_URL
fi
# The configs the installer puts in the new user's home (compositor, Noctalia, fish with tide): this project's repository.
[ -n "\${GENTOO_WM_CONFIGS_URL:-}" ] || export GENTOO_WM_CONFIGS_URL=$CONFIGS_URL
SHENV
if [ -n "$STORE_BINHOST" ] && [ -n "$STORE_OVERLAY" ]; then
    cat >> "$ROOT/etc/profile.d/installer-stage.sh" <<SHSTORE
[ -n "\${GENTOO_STORE_BINHOST_URL:-}" ] || export GENTOO_STORE_BINHOST_URL=$STORE_BINHOST
[ -n "\${GENTOO_STORE_OVERLAY_URL:-}" ] || export GENTOO_STORE_OVERLAY_URL=$STORE_OVERLAY
SHSTORE
fi
cat > "$ROOT/etc/fish/conf.d/20-installer-stage.fish" <<FISHENV
# Written by prepare-rootfs.sh: where the installer gets its stage3 when the medium carries none.
if not set -q GENTOO_INSTALLER_STAGE3_URL; and not test -d /run/initramfs/live/stage; and not test -d /run/live/medium/stage
    set -gx GENTOO_INSTALLER_STAGE3_URL $STAGE_URL
end
set -q GENTOO_WM_CONFIGS_URL; or set -gx GENTOO_WM_CONFIGS_URL $CONFIGS_URL
FISHENV
if [ -n "$STORE_BINHOST" ] && [ -n "$STORE_OVERLAY" ]; then
    cat >> "$ROOT/etc/fish/conf.d/20-installer-stage.fish" <<FISHSTORE
set -q GENTOO_STORE_BINHOST_URL; or set -gx GENTOO_STORE_BINHOST_URL $STORE_BINHOST
set -q GENTOO_STORE_OVERLAY_URL; or set -gx GENTOO_STORE_OVERLAY_URL $STORE_OVERLAY
FISHSTORE
fi

# Start OpenRC services in parallel: the boot is mostly waiting for file reads (decompressing the root image), and several
# services reading at once keep more CPUs busy decompressing (measured with the EROFS image, see the iso README).
sed -i 's/^#\?rc_parallel=.*/rc_parallel="YES"/' "$ROOT/etc/rc.conf"
grep -q '^rc_parallel="YES"' "$ROOT/etc/rc.conf" || echo 'rc_parallel="YES"' >> "$ROOT/etc/rc.conf"

echo gentoo-live > "$ROOT/etc/hostname"
sed -i 's/^hostname=.*/hostname="gentoo-live"/' "$ROOT/etc/conf.d/hostname"

# Network: dhcpcd for wired links, iwd for Wi-Fi (the installer's network page talks to iwd).
chroot "$ROOT" rc-update add iwd default >/dev/null 2>&1 || true
chroot "$ROOT" rc-update add dhcpcd default >/dev/null 2>&1 || true

# The live system never mounts anything from fstab; an empty one avoids fsck/remount noise.
: > "$ROOT/etc/fstab"
