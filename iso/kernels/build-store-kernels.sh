#!/bin/bash
# build-store-kernels.sh <kernel-src> <kernel-configs-dir> <work-dir> <store-dir> TARGET...   (run as root)
#
# Builds the given targets of the kernel matrix (`kernel-configs/matrix-build.sh`, its own scripts, seed and
# fragments) and lays the results out the way the installer's `kernel::deploy` fetches them:
#
#   <store>/kernels/<combo>.vmlinuz
#   <store>/kernels/<combo>-modules.tar.xz     (contains <kernel-release>/…, extracted into /lib/modules)
#   <store>/kernels/<combo>-devel.tar.xz       (NVIDIA combos only: what `x11-drivers/nvidia-drivers` builds against)
#   <store>/Packages                           (`CPV: sys-kernel/gentoo-diy-kernel-bin-<combo>-1`, what `store::list_binhost_atoms` reads)
#
# <combo> is the target name without its `pop-NNN-` rank, e.g. pop-003-intel-raptorlake-nvidia-laptop →
# intel-raptorlake-nvidia-laptop. Targets that fail to build are reported and left out of `Packages`.
set -uo pipefail
SRC=${1:?kernel source tree}; CFG=${2:?kernel-configs dir}; WORK=${3:?work dir}; STORE=${4:?store dir}; shift 4
[ $# -gt 0 ] || { echo "no targets given" >&2; exit 2; }
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# sccache in front of gcc destroys /dev/null (see cc-sccache-safe); matrix-build.sh honours $CC.
export CC="${CC:-$HERE/cc-sccache-safe}"
devnull_ok() { [ -c /dev/null ] || { echo "WARNING: /dev/null was not a character device; restoring it" >&2; rm -f /dev/null; mknod -m 666 /dev/null c 1 3; }; }
mkdir -p "$WORK" "$STORE/kernels"
touch "$STORE/Packages.new"; : > "$STORE/Packages.new"
for t in "$@"; do
    combo=$(echo "$t" | sed -E 's/^pop-[0-9]+-//')
    devnull_ok
    echo "=== $t -> $combo ($(date +%H:%M))"
    nice -n 15 "$CFG/matrix-build.sh" "$SRC" "$WORK" --target "$t" --build --yes --parallel 1 || true
    devnull_ok
    d="$WORK/$t"; bz="$d/srctree/arch/x86/boot/bzImage"; mods="$d/srctree/modules_root/lib/modules"
    if [ ! -f "$bz" ] || [ ! -d "$mods" ]; then echo "FAILED: $t (no bzImage or modules; see $d/build.log)" >&2; continue; fi
    cp "$bz" "$STORE/kernels/$combo.vmlinuz"
    tar -C "$mods" -cf - . | xz -T0 -6 > "$STORE/kernels/$combo-modules.tar.xz"
    [ -f "$d/$t-devel.tar.xz" ] && cp "$d/$t-devel.tar.xz" "$STORE/kernels/$combo-devel.tar.xz"
    printf 'CPV: sys-kernel/gentoo-diy-kernel-bin-%s-1\n\n' "$combo" >> "$STORE/Packages.new"
    ls -l "$STORE/kernels/$combo.vmlinuz" | awk '{printf "ok %s %.0f MB\n", $9, $5/1048576}'
    # the per-target source copy is several GB; the products are in the store now
    [ -d "$d/srctree" ] && [ "${KEEP_TREES:-0}" != 1 ] && rm -rf --one-file-system "$d/srctree"
done
# merge with what the store already lists
cat "$STORE/Packages" "$STORE/Packages.new" 2>/dev/null | awk 'BEGIN{RS="";ORS="\n\n"} !seen[$0]++' > "$STORE/Packages.merged" && mv "$STORE/Packages.merged" "$STORE/Packages"
rm -f "$STORE/Packages.new"; echo "done: $(grep -c '^CPV' "$STORE/Packages") packages in $STORE/Packages"
