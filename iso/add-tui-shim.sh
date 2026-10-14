#!/bin/bash
# add-tui-shim.sh <rootfs>   (run as root, in a mount namespace like the other rootfs steps; the GUI rootfs only)
# Emerges app-misc/tui-shim (launchers for terminal programs, and the file-picker badge for micro and nvim) into a live rootfs from its
# live ebuild, with the ebuild in a small overlay inside the rootfs, plus gui-apps/grim (the badge finder). Needs network for the crates.
# The tui-shim repository has no public address yet, so the source is a snapshot of the local checkout ($TUI_SHIM_SRC, default: the
# tui-shim checkout next to this repository) copied into the rootfs for the build and removed afterwards. Rust is build-time only.
set -euo pipefail
R=$(readlink -f "${1:?rootfs}")
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SRC=${TUI_SHIM_SRC:-$(cd "$HERE/../.." && pwd)/tui-shim}
EB=$SRC/packaging/gentoo/app-misc/tui-shim
[ -f "$EB/tui-shim-9999.ebuild" ] || { echo "add-tui-shim.sh: no tui-shim ebuild in $EB" >&2; exit 1; }
[ -z "$(git -c safe.directory='*' -C "$SRC" status --porcelain)" ] || { echo "add-tui-shim.sh: $SRC has uncommitted changes" >&2; exit 1; }

OV=$R/var/db/repos/tui-shim-overlay
rm -rf "$OV"; mkdir -p "$OV/profiles" "$OV/metadata" "$OV/app-misc/tui-shim"
echo tui-shim-overlay > "$OV/profiles/repo_name"
printf 'masters = gentoo\n' > "$OV/metadata/layout.conf"
cp "$EB"/tui-shim-9999.ebuild "$EB"/metadata.xml "$OV/app-misc/tui-shim/"
printf '[tui-shim-overlay]\nlocation = /var/db/repos/tui-shim-overlay\n' > "$R/etc/portage/repos.conf/tui-shim.conf"
mkdir -p "$R/etc/portage/package.accept_keywords" "$R/etc/portage/package.use"
echo 'app-misc/tui-shim **' > "$R/etc/portage/package.accept_keywords/tui-shim"

# a clone of the checkout for git-r3 to read (the .git directory is what counts)
rm -rf "$R/tmp/tui-shim-src"; git -c safe.directory='*' clone -q "$SRC" "$R/tmp/tui-shim-src"

mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
cp /etc/resolv.conf "$R/etc/resolv.conf"
trap ': > "$R/etc/resolv.conf"; rm -rf "$R/tmp/tui-shim-src"' EXIT
chroot "$R" env LC_ALL=C.UTF-8 MAKEOPTS=-j16 EGIT_OVERRIDE_REPO_TARILKA0GG_TUI_SHIM=file:///tmp/tui-shim-src bash -c '
    (cd /var/db/repos/tui-shim-overlay/app-misc/tui-shim && ebuild ./tui-shim-9999.ebuild manifest >/dev/null 2>&1)
    emerge -q --noreplace gui-apps/grim 2>&1 | tail -5
    emerge -q app-misc/tui-shim 2>&1 | tail -25'
chroot "$R" bash -c 'command -v tui-shim grim'
