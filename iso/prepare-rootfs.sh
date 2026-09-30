#!/bin/bash
# prepare-rootfs.sh <rootfs> <installer-cli-binary> [installer-gui-binary gentoo-wm-configs-dir [profile-dir]]
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

# Empty root password + autologin on both consoles. The serial line is what QEMU tests use.
chroot "$ROOT" passwd -d root >/dev/null
sed -i -E '/^c1:/d; /^s0:/d' "$ROOT/etc/inittab"
cat >> "$ROOT/etc/inittab" <<'INITTAB'
c1:12345:respawn:/sbin/agetty --autologin root --noclear 38400 tty1 linux
s0:12345:respawn:/sbin/agetty --autologin root -L 115200 ttyS0 vt100
INITTAB
grep -q '^ttyS0$' "$ROOT/etc/securetty" 2>/dev/null || echo ttyS0 >> "$ROOT/etc/securetty"

# Start the installer on the first console that logs in; leaving it drops to a shell.
# With a GUI binary, tty1 starts the user's own niri + Noctalia session (config from the
# gentoo-wm-configs repo) with installer-gui launched on top, full screen; if the session
# ends, the text installer takes over. The serial console always gets the TUI.
if [ -n "$GUI" ]; then
    install -Dm755 "$GUI" "$ROOT/usr/local/bin/installer-gui"
    [ -n "$WMCONF" ] || { echo "GUI mode needs the gentoo-wm-configs directory" >&2; exit 1; }
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
window-rule {
    match app-id="org.gentoo_diy.Installer"
    open-floating true
}
NIRILIVE
    cat > "$ROOT/root/.bash_profile" <<'PROFILE'
if [ -z "${INSTALLER_STARTED:-}" ]; then
    export INSTALLER_STARTED=1
    case "$(tty)" in
    /dev/tty1)
        export XDG_RUNTIME_DIR=/run/user/0 LIBSEAT_BACKEND=seatd
        mkdir -p -m 700 "$XDG_RUNTIME_DIR"
        # niri refuses software rendering and needs a real output. If no output exists 25 s
        # in (no hardware GL, e.g. a VM without 3D), stop it rather than leave a black screen.
        rm -f /run/live-gui-failed
        (
            sleep 25
            sock=$(ls /run/user/0/niri.*.sock 2>/dev/null | head -1)
            if [ -z "$sock" ] || [ -z "$(NIRI_SOCKET=$sock niri msg outputs 2>/dev/null)" ]; then
                touch /run/live-gui-failed
                pkill -x niri
            fi
        ) &
        dbus-run-session -- niri --session 2>/var/log/niri-session.log
        if [ -e /run/live-gui-failed ]; then
            echo "No usable graphics output (niri needs hardware-accelerated graphics)."
            echo "Log: /var/log/niri-session.log. Starting the text installer."
        else
            echo "The graphical session ended; starting the text installer."
        fi
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
        set -gx XDG_RUNTIME_DIR /run/user/0
        set -gx LIBSEAT_BACKEND seatd
        mkdir -p -m 700 $XDG_RUNTIME_DIR
        # niri needs a real output; if there is none after 25 s (no hardware GL), stop it.
        rm -f /run/live-gui-failed
        begin
            sleep 25
            set -l sock (ls /run/user/0/niri.*.sock 2>/dev/null | head -1)
            if test -z "$sock"; or test -z (env NIRI_SOCKET=$sock niri msg outputs 2>/dev/null | string collect)
                touch /run/live-gui-failed
                pkill -x niri
            end
        end &
        dbus-run-session -- niri --session 2>/var/log/niri-session.log
        if test -e /run/live-gui-failed
            echo "No usable graphics output (niri needs hardware-accelerated graphics)."
            echo "Log: /var/log/niri-session.log. Starting the text installer."
        else
            echo "The graphical session ended; starting the text installer."
        end
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

echo gentoo-live > "$ROOT/etc/hostname"
sed -i 's/^hostname=.*/hostname="gentoo-live"/' "$ROOT/etc/conf.d/hostname"

# Network: dhcpcd for wired links, iwd for Wi-Fi (the installer's network page talks to iwd).
chroot "$ROOT" rc-update add iwd default >/dev/null 2>&1 || true
chroot "$ROOT" rc-update add dhcpcd default >/dev/null 2>&1 || true

# The live system never mounts anything from fstab; an empty one avoids fsck/remount noise.
: > "$ROOT/etc/fstab"
