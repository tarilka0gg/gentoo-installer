#!/bin/bash
# refresh-rootfs.sh <work-dir> <rootfs-name> [gui]      (run as root)
# Puts the current kernel modules into <work>/<rootfs>, (re)applies prepare-rootfs.sh and
# regenerates the dracut initramfs. "gui" = also install installer-gui and the niri profile.
# Expects: <work>/modroot (make modules_install INSTALL_MOD_PATH), the release binaries in
# $INSTALLER_TARGET_DIR (default ../target/release), the wm-configs repo in $WM_CONFIGS and,
# optionally, a make-profile.py output in $PROFILE and a wallpaper set in $WALLPAPERS
# and GTK theme/icon directories in $THEME_ASSETS (icons/, themes/).
set -euo pipefail
W=$(readlink -f "${1:?work dir}"); NAME=${2:?rootfs name}; MODE=${3:-}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
BIN=${INSTALLER_TARGET_DIR:-$HERE/../target/release}
WM_CONFIGS=${WM_CONFIGS:-$(cd "$HERE/../.." && pwd)/simple-linux-configs}   # sibling checkout
R=$W/$NAME
KVER=$(ls "$W/modroot/lib/modules")

cat > "$W/refresh-inner.sh" <<INNER
set -e
mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
cp /etc/resolv.conf "$R/etc/resolv.conf"
rm -rf "$R/lib/modules/$KVER"; mkdir -p "$R/lib/modules"
cp -a "$W/modroot/lib/modules/$KVER" "$R/lib/modules/"
if [ "$MODE" = gui ]; then
    bash "$HERE/prepare-rootfs.sh" "$R" "$BIN/installer-cli" "$BIN/installer-gui" "$WM_CONFIGS" ${PROFILE:-}
else
    bash "$HERE/prepare-rootfs.sh" "$R" "$BIN/installer-cli"
fi
chroot "$R" depmod "$KVER"
rm -rf "$R/usr/lib/dracut/modules.d/76iso-autoscan"
cp -a "$HERE/dracut/76iso-autoscan" "$R/usr/lib/dracut/modules.d/"
# Early microcode: stored uncompressed in front of the initramfs, and the ISO carries the initramfs
# twice (boot/ and the UEFI image), so it costs ~80 MB — but without it the kernel runs on whatever
# the firmware loaded ("TSC_DEADLINE disabled ... please update microcode" on the first real test).
bash "$HERE/prune-microcode.sh" "$R"
chroot "$R" env LC_ALL=C.UTF-8 dracut --force --no-hostonly --early-microcode --kver "$KVER" \
    --add "dmsquash-live iso-autoscan" --omit "plymouth nfs iscsi multipath crypt lvm mdraid" --compress zstd /boot/initramfs-live.img
INNER
unshare --mount --propagation private bash "$W/refresh-inner.sh"
rm -f "$W/refresh-inner.sh"
