#!/bin/bash
# slim-rootfs.sh <rootfs>     (run as root; compiles for a while)
# Puts a live rootfs on the `live` flavor of the Simple Linux profiles (profiles/flavor/live): `slp` writes the Portage configuration of
# the flavor into the rootfs (package.use) and the packages whose USE flags it changes are rebuilt. What the flavor says is data in
# profiles/, not in this script:
#   * Mesa without LLVM: the AMD driver builds with its own compiler (ACO) and software GL is softpipe instead of llvmpipe, so
#     libLLVM (107 MiB, 27 MiB compressed) is left out of the image,
#   * git without Perl, Python and GnuPG: git's Perl scripts are what held the Perl tree in the image,
#   * tcpdump without libsmi (its MIB files are 17 MiB).
# The rootfs is changed in place: clone it first with `cp -a --reflink=always` (btrfs) if you want to keep the old one. Cores:
# `taskset -c 0-7 nice -n 10 slim-rootfs.sh <rootfs>` on a laptop. `SLP` names the slp binary (default: target/release/slp).
set -euo pipefail
R=$(readlink -f "${1:?rootfs}")
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SLP=${SLP:-$HERE/../target/release/slp}
[ "$(id -u)" = 0 ] || { echo "slim-rootfs.sh: run as root" >&2; exit 1; }
[ -x "$SLP" ] || { echo "slim-rootfs.sh: $SLP is missing (cargo build --release -p slp)" >&2; exit 1; }

# the hand-written files this script and the profiles replaced
rm -f "$R/etc/portage/package.use/zz-slim" "$R/etc/portage/profile/package.use.force/zz-slim"
if [ -f "$R/etc/slp/state" ]; then "$SLP" --root "$R" flavor live; else "$SLP" --root "$R" init live; fi
mapfile -t REBUILD < <("$SLP" --root "$R" rebuild)
[ "${#REBUILD[@]}" -gt 0 ] || { echo "slim-rootfs.sh: the live flavor has nothing to rebuild" >&2; exit 0; }
echo "rebuilding: ${REBUILD[*]}"
unshare --mount --propagation private bash -c '
R=$1; shift
mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
cp /etc/resolv.conf "$R/etc/resolv.conf"
exec chroot "$R" /usr/bin/env -i HOME=/root TERM=xterm PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin LC_ALL=C.UTF-8 MAKEOPTS="-j$(nproc)" \
    emerge -1 --oneshot "$@"
' _ "$R" "${REBUILD[@]}"
