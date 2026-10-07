#!/bin/bash
# build-limine-bios.sh <out-dir>
# Builds Limine's BIOS stages (limine-bios-cd.bin, limine-bios.sys, limine-bios-hdd.bin and the `limine` tool) with
# patches/limine-12.3.3-ventoy-bios.patch, which makes a 2048-byte-sector drive count as an optical drive. Ventoy's virtual
# CD is such a drive but is neither flagged removable nor ATAPI, so stock Limine skips it and stops with "Could not determine
# boot drive" when the ISO is started from Ventoy in normal mode on BIOS. Use the result with
#   LIMINE_BIOS_DIR=<out-dir> LIMINE_TOOL=<out-dir>/limine ./assemble-iso.sh ...
# Needs clang + lld (LLVM_BIN, default /usr/lib/llvm/22/bin), nasm, mtools, curl.
set -euo pipefail
OUT=$(mkdir -p "${1:?out dir}" && readlink -f "$1")
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
V=12.3.3
T=$(mktemp -d); trap 'rm -rf "${T:?}"' EXIT
curl -fsSL -o "$T/l.tar.xz" "https://github.com/Limine-Bootloader/Limine/releases/download/v$V/limine-$V.tar.xz"
tar -C "$T" -xf "$T/l.tar.xz"
patch -d "$T/limine-$V" -p1 < "$HERE/patches/limine-$V-ventoy-bios.patch"
mkdir "$T/b"; cd "$T/b"
export PATH=${LLVM_BIN:-/usr/lib/llvm/22/bin}:$PATH
"../limine-$V/configure" --enable-bios --enable-bios-cd >/dev/null
make -j"$(nproc)" >/dev/null
cp bin/limine-bios-cd.bin bin/limine-bios.sys bin/limine-bios-hdd.bin bin/limine "$OUT/"
echo "built Limine $V BIOS stages (Ventoy patch) in $OUT"
