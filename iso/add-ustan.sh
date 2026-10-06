#!/bin/bash
# add-ustan.sh <rootfs> gui|cli   (run as root, in a mount namespace like the other rootfs steps)
# Emerges app-misc/ustan (Windows-style installer for .deb/AppImage/Flatpak/.exe) into a live rootfs from its live ebuild
# (https://github.com/tarilka0gg/ustan), with the ebuild in a small overlay inside the rootfs. Needs network. `gui` builds
# ustan-gui too (GTK4/libadwaita are in the GUI rootfs); `cli` only the command-line tool (the minimal image has no GTK).
# Rust and Zig are build-time only: the rootfs has them under /opt and assemble-iso.sh leaves them out of the image.
# $USTAN_EBUILD_DIR is where the ebuild and metadata.xml are (default: the ustan checkout next to this repository).
set -euo pipefail
R=$(readlink -f "${1:?rootfs}"); MODE=${2:?gui|cli}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SRC=${USTAN_EBUILD_DIR:-$(cd "$HERE/../.." && pwd)/ustan/packaging/gentoo/app-misc/ustan}
[ -f "$SRC/ustan-9999.ebuild" ] || { echo "add-ustan.sh: no ustan ebuild in $SRC" >&2; exit 1; }
case $MODE in gui) USEFLAG=gui ;; cli) USEFLAG=-gui ;; *) echo "mode must be gui or cli" >&2; exit 2 ;; esac

OV=$R/var/db/repos/ustan-overlay
rm -rf "$OV"; mkdir -p "$OV/profiles" "$OV/metadata" "$OV/app-misc/ustan"
echo ustan-overlay > "$OV/profiles/repo_name"
printf 'masters = gentoo\n' > "$OV/metadata/layout.conf"
cp "$SRC"/ustan-9999.ebuild "$SRC"/metadata.xml "$OV/app-misc/ustan/"
printf '[ustan-overlay]\nlocation = /var/db/repos/ustan-overlay\n' > "$R/etc/portage/repos.conf/ustan.conf"
mkdir -p "$R/etc/portage/package.accept_keywords" "$R/etc/portage/package.use"
echo 'app-misc/ustan **' > "$R/etc/portage/package.accept_keywords/ustan"
echo "app-misc/ustan $USEFLAG" > "$R/etc/portage/package.use/ustan"
# x11-misc/xdg-utils (a dependency) wants xmlto built with the `text` flag in a rootfs that has not got it yet.
echo "app-text/xmlto text" >> "$R/etc/portage/package.use/ustan"

mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
cp /etc/resolv.conf "$R/etc/resolv.conf"
trap ': > "$R/etc/resolv.conf"' EXIT
chroot "$R" env LC_ALL=C.UTF-8 MAKEOPTS=-j16 bash -c '
    (cd /var/db/repos/ustan-overlay/app-misc/ustan && ebuild ./ustan-9999.ebuild manifest >/dev/null 2>&1)
    # The ebuild accepts dev-lang/zig or zig-bin; take the binary one first, or Portage would compile Zig and LLVM from source.
    if ! command -v zig >/dev/null 2>&1; then
        echo "dev-lang/zig-bin ~amd64" >> /etc/portage/package.accept_keywords/ustan
        emerge -q --noreplace dev-lang/zig-bin 2>&1 | tail -5
    fi
    emerge -q --noreplace app-misc/ustan 2>&1 | tail -25'
chroot "$R" bash -c 'command -v ustan && ls /usr/bin/ustan-gui 2>/dev/null; true'
