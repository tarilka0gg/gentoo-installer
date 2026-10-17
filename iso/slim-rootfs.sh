#!/bin/bash
# slim-rootfs.sh <rootfs>     (run as root; compiles for a while)
# Rebuilds the packages of a live rootfs whose USE flags make the image bigger than it needs to be, with a configuration the live image
# keeps (so a rootfs rebuilt from scratch gets the same result):
#   * LLVM for the CPU targets a PC uses (X86, AMDGPU) only. Mesa's software renderer and radeonsi load libLLVM, which with all of
#     LLVM's targets is the biggest library of the image. The profile forces the targets on, hence the package.use.force lines.
#   * git without Perl, Python and GnuPG: git's Perl scripts are what held the Perl tree in the image.
#   * tcpdump without libsmi (its MIB files are 17 MiB).
# Cores: `taskset -c 0-7 nice -n 10 slim-rootfs.sh <rootfs>` on a laptop. The rootfs is changed in place: clone it first with
# `cp -a --reflink=always` (btrfs) if you want to keep the old one.
set -euo pipefail
R=$(readlink -f "${1:?rootfs}")
[ "$(id -u)" = 0 ] || { echo "slim-rootfs.sh: run as root" >&2; exit 1; }
P=$R/etc/portage
grep -q '^LLVM_TARGETS=' "$P/make.conf" || printf '\n# live image: only the CPU targets a PC needs (LLVM is the biggest library of the image)\nLLVM_TARGETS="X86 AMDGPU"\n' >> "$P/make.conf"
cat > "$P/package.use/zz-slim" <<'USE'
# the live image does not need these (see iso/slim-rootfs.sh)
dev-vcs/git -perl -python -gpg -nls
net-analyzer/tcpdump -smi
USE
mkdir -p "$P/profile/package.use.force"
: > "$P/profile/package.use.force/zz-slim"
for p in llvm clang lld; do
    printf 'llvm-core/%s' "$p" >> "$P/profile/package.use.force/zz-slim"
    for t in AArch64 ARM AVR BPF Hexagon Lanai LoongArch MSP430 Mips NVPTX PowerPC RISCV SPIRV Sparc SystemZ VE WebAssembly XCore; do
        printf ' -llvm_targets_%s' "$t" >> "$P/profile/package.use.force/zz-slim"
    done
    echo >> "$P/profile/package.use.force/zz-slim"
done
unshare --mount --propagation private bash -c '
R=$1
mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
cp /etc/resolv.conf "$R/etc/resolv.conf"
exec chroot "$R" /usr/bin/env -i HOME=/root TERM=xterm PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin LC_ALL=C.UTF-8 MAKEOPTS="-j$(nproc)" \
    emerge -1 --oneshot llvm-core/llvm llvm-core/clang llvm-core/lld dev-vcs/git net-analyzer/tcpdump
' _ "$R"
