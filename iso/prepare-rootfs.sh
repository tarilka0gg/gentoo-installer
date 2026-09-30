#!/bin/bash
# prepare-rootfs.sh <rootfs> <installer-cli-binary>
# Turns an unpacked stage3 (with the runtime tools already emerged) into the live system:
# root autologin on tty1 + serial, the installer started from root's login shell, iwd and
# dhcpcd on boot, an empty root password (this is a live ISO, not an installed system).
set -euo pipefail
ROOT=${1:?rootfs}
CLI=${2:?installer-cli binary}

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
cat > "$ROOT/root/.bash_profile" <<'PROFILE'
if [ -z "${INSTALLER_STARTED:-}" ] && { [ "$(tty)" = /dev/tty1 ] || [ "$(tty)" = /dev/ttyS0 ]; }; then
    export INSTALLER_STARTED=1
    installer-cli
    echo "Installer exited. This is a live shell; run 'installer-cli' to start it again."
fi
PROFILE

echo gentoo-live > "$ROOT/etc/hostname"
sed -i 's/^hostname=.*/hostname="gentoo-live"/' "$ROOT/etc/conf.d/hostname"

# Network: dhcpcd for wired links, iwd for Wi-Fi (the installer's network page talks to iwd).
chroot "$ROOT" rc-update add iwd default >/dev/null 2>&1 || true
chroot "$ROOT" rc-update add dhcpcd default >/dev/null 2>&1 || true

# The live system never mounts anything from fstab; an empty one avoids fsck/remount noise.
: > "$ROOT/etc/fstab"
