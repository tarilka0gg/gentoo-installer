#!/bin/bash
# prune-microcode.sh <rootfs>  - drops the Intel microcode of CPU families that are server-only and the newest, before dracut packs it.
# The early-microcode cpio holds every file of /usr/lib/firmware/intel-ucode uncompressed (37 MB) and the ISO carries the initramfs twice, so the
# biggest files cost the most: 06-ad/ae (Granite Rapids), 06-af (Sierra Forest), 06-dd, 06-b6 (Grand Ridge), 06-cf (Emerald Rapids) are about
# 14 of the 37 MB. A CPU without its file simply runs the firmware's microcode (it still boots; only later mitigations are missing).
# 06-8f (Sapphire Rapids, also the Xeon W workstations) stays. Files with identical content are kept once. KEEP_MICROCODE=all keeps everything.
set -euo pipefail
ROOT=$(readlink -f "${1:?rootfs}")
[ "${KEEP_MICROCODE:-}" = all ] && exit 0
D=$ROOT/usr/lib/firmware/intel-ucode
[ -d "$D" ] || exit 0
before=$(du -sm "$D" | cut -f1)
for model in ad ae af dd b6 cf; do rm -f "$D"/06-"$model"-*; done
# Many files are the same blob under several CPU signatures (Sapphire Rapids alone four times, 600 KB each) and dracut packs every file:
# the early-microcode cpio only needs each distinct blob once (the kernel matches by the signature inside the blob, not by the file name).
declare -A seen
for f in $(ls "$D" | sort); do
    h=$(sha256sum "$D/$f" | cut -d" " -f1)
    if [ -n "${seen[$h]:-}" ]; then rm -f "$D/$f"; else seen[$h]=1; fi
done
echo "prune-microcode: Intel microcode $before MB -> $(du -sm "$D" | cut -f1) MB" >&2
