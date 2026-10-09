#!/bin/bash
# build-variant.sh <work-dir> "<VIDEO_CARDS>" <atom>...      (run as root; the work dir of an earlier build.sh run)
# Builds another instance of a package with a different VIDEO_CARDS, next to the one already in the binhost. Portage keeps both
# in the index (same version, different USE, own build id) and a client takes the one whose flags match its own. The installer asks
# for VIDEO_CARDS = the binhost's set plus what the hardware needs (`virgl` in a QEMU/virtio-gpu VM), so
#   build-variant.sh <work> "amdgpu intel nouveau radeon radeonsi virgl" media-libs/mesa
# makes mesa a download in a VM too, instead of ~6 minutes of compiling.
# Cores are not pinned here: `taskset -c 0-15 nice -n 10 build-variant.sh ...` on the laptop.
set -euo pipefail
W=$(readlink -f "${1:?work dir}"); CARDS=${2:?VIDEO_CARDS}; shift 2; [ $# -gt 0 ] || { echo "no atom given" >&2; exit 2; }
R=$W/root; MC=$R/etc/portage/make.conf; JOBS=$(nproc)
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
[ -f "$MC" ] || { echo "$W is not the work dir of a build" >&2; exit 1; }
cp "$MC" "$W/make.conf.before-variant"
trap 'cp "$W/make.conf.before-variant" "$MC"' EXIT      # the main configuration comes back whatever happens
sed -i -E "s|^VIDEO_CARDS=.*|VIDEO_CARDS=\"$CARDS\"|; s|^MAKEOPTS=.*|MAKEOPTS=\"-j$JOBS\"|" "$MC"
grep -q '^VIDEO_CARDS=' "$MC" || echo "VIDEO_CARDS=\"$CARDS\"" >> "$MC"
exclude=(); for a in "$@"; do exclude+=(--usepkg-exclude "$a"); done
cat > "$R/root/variant-inner.sh" <<INNER
set -e
export LC_ALL=C.UTF-8
mkdir -p /run/lock
# -1: not added to world; the package is already installed with the other flags, so it is rebuilt for the new ones
CONFIG_PROTECT_MASK=/etc/portage emerge -1v --getbinpkg --usepkg --buildpkg ${exclude[*]} $* 
emaint binhost --fix >/dev/null 2>&1 || true
INNER
cat > "$W/ns-variant.sh" <<NS
set -e
mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
[ "\${BUILD_TMPFS:-1}" = 0 ] || mount -t tmpfs -o size=6g tmpfs "$R/var/tmp"
printf "nameserver 1.1.1.1\nnameserver 8.8.8.8\n" > "$R/etc/resolv.conf"
exec chroot "$R" /bin/bash /root/variant-inner.sh
NS
mkdir -p "$W/out"
unshare --mount --propagation private bash "$W/ns-variant.sh" 2>&1 | tee "$W/out/variant.log"
: > "$R/etc/resolv.conf"
echo "variant built; run make-repo.sh to refresh the repository directory"
