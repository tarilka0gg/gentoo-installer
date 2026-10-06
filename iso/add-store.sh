#!/bin/bash
# add-store.sh <rootfs>   (run as root, in a mount namespace like the other rootfs steps; the GUI rootfs only)
# Emerges app-portage/portage-store (GTK4 app store for Portage) into a live rootfs from its live ebuild
# (https://github.com/tarilka0gg/portage-store), with the ebuild in a small overlay inside the rootfs. Needs network.
# Rust is build-time only: the rootfs has it under /opt and assemble-iso.sh leaves it out of the image.
# $STORE_EBUILD_DIR is where the ebuild and metadata.xml are (default: the portage-store checkout next to this repository).
set -euo pipefail
R=$(readlink -f "${1:?rootfs}")
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SRC=${STORE_EBUILD_DIR:-$(cd "$HERE/../.." && pwd)/portage-store/packaging/gentoo/app-portage/portage-store}
[ -f "$SRC/portage-store-9999.ebuild" ] || { echo "add-store.sh: no portage-store ebuild in $SRC" >&2; exit 1; }

OV=$R/var/db/repos/store-overlay
rm -rf "$OV"; mkdir -p "$OV/profiles" "$OV/metadata" "$OV/app-portage/portage-store"
echo store-overlay > "$OV/profiles/repo_name"
printf 'masters = gentoo\n' > "$OV/metadata/layout.conf"
cp "$SRC"/portage-store-9999.ebuild "$SRC"/metadata.xml "$OV/app-portage/portage-store/"
printf '[store-overlay]\nlocation = /var/db/repos/store-overlay\n' > "$R/etc/portage/repos.conf/store.conf"
mkdir -p "$R/etc/portage/package.accept_keywords" "$R/etc/portage/package.use"
echo 'app-portage/portage-store **' > "$R/etc/portage/package.accept_keywords/portage-store"
echo 'app-portage/portage-store -webview' > "$R/etc/portage/package.use/portage-store"

mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
cp /etc/resolv.conf "$R/etc/resolv.conf"
trap ': > "$R/etc/resolv.conf"' EXIT
chroot "$R" env LC_ALL=C.UTF-8 MAKEOPTS=-j16 bash -c '
    (cd /var/db/repos/store-overlay/app-portage/portage-store && ebuild ./portage-store-9999.ebuild manifest >/dev/null 2>&1)
    emerge -q --noreplace app-portage/portage-store 2>&1 | tail -25'
chroot "$R" bash -c 'for f in /usr/bin/portage-store /usr/bin/portage-store-cli /usr/libexec/portage-store/priv-helper /usr/share/applications/io.github.tarilka0gg.PortageStore.desktop; do ls $f 2>&1; done'
