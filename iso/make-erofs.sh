#!/bin/bash
# make-erofs.sh <rootfs> <excludes-file> <out.erofs>   (run as root; needs erofs-utils: mkfs.erofs)
# The live root image. EROFS with LZMA and 1 MiB clusters came out 14 % smaller than squashfs/zstd-19 (740 vs 859 MiB for the GUI
# tree) and builds faster (about a minute); the price is about +2 s of boot (lzma decompresses slower than zstd). It is built from a
# directory, not a tar: mkfs.erofs 1.8.10 given a tar together with fragdedupe=inode silently stored almost nothing.
# The kernel needs CONFIG_EROFS_FS (+ _ZIP_LZMA); dracut's dmsquash-live finds the image type by itself.
set -euo pipefail
ROOTFS=$(readlink -f "${1:?rootfs}"); EXCL=$(readlink -f "${2:?excludes file}"); OUT=$(readlink -m "${3:?out image}")
command -v mkfs.erofs >/dev/null || { echo "make-erofs.sh: mkfs.erofs not found (emerge sys-fs/erofs-utils)" >&2; exit 1; }
TREE=$(mktemp -d "${OUT}.tree.XXXXXX"); trap 'rm -rf "${TREE:?}"' EXIT
# The same exclusion list mksquashfs uses; tar wants the paths anchored at ./ . Device nodes and fifos stay (we are root).
sed 's#^#./#' "$EXCL" > "$TREE.ex"
tar -C "$ROOTFS" --anchored --no-wildcards --exclude-from="$TREE.ex" --numeric-owner --xattrs -cf - . | tar -C "$TREE" --numeric-owner --xattrs -xpf -
rm -f "$TREE.ex" "$OUT"
mkfs.erofs -zlzma,9 -C1048576 -Eall-fragments,fragdedupe=inode "$OUT" "$TREE" >/dev/null
echo "$OUT"
