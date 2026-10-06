#!/bin/bash
# add-tools.sh <rootfs>   (run as root in a mount namespace, needs network)
# The tools every Simple Linux image carries, in step with the installer's "tools" group: btop (it replaces htop, which is removed) and fastfetch.
set -euo pipefail
R=$(readlink -f "${1:?rootfs}")
mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
cp /etc/resolv.conf "$R/etc/resolv.conf"
trap ': > "$R/etc/resolv.conf"' EXIT
chroot "$R" env LC_ALL=C.UTF-8 MAKEOPTS=-j16 bash -c '
    emerge -q --noreplace sys-process/btop app-misc/fastfetch 2>&1 | tail -4
    emerge -q -C sys-process/htop 2>&1 | tail -2'
chroot "$R" bash -c 'for p in btop fastfetch htop; do printf "%s: " $p; command -v $p || echo absent; done'
