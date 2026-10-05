#!/bin/bash
# run-gl-vm.sh <work-dir> <iso> — boot an ISO in a QEMU with a 3D virtual GPU (virgl), as root.
#
# The distro QEMU usually has no GL display, and niri refuses software rendering, so the GUI image can
# only be shown working on a virtual GPU. <work-dir>/qemu-gl is a stage3 copy with
# `app-emulation/qemu[opengl,virgl]` (x86_64 target only) emerged into it; the host is left alone.
# Serial is <work-dir>/ser.sock, the monitor <work-dir>/gui.sock (screendump does NOT work with GL
# scanout — take pictures inside the guest, see guest-shot.py). The iso must sit in <work-dir>.
# Needs a render node: RENDERNODE=/dev/dri/renderD128 (default renderD129 is the author's iGPU).
set -euo pipefail
W=$(readlink -f "${1:?work dir}"); ISO=$(basename "${2:?iso}")
R=$W/qemu-gl
rm -f "$W/ser.sock" "$W/gui.sock"
mkdir -p "$R/mnt/work"
exec unshare --mount --propagation private bash -c "
mount --bind /proc $R/proc; mount --rbind /sys $R/sys; mount --rbind /dev $R/dev; mount --bind $W $R/mnt/work
exec taskset -c 0-15 chroot $R qemu-system-x86_64 -machine q35 -enable-kvm -cpu host -m 4096 -smp 4 \
  -device virtio-vga-gl -display egl-headless,rendernode=${RENDERNODE:-/dev/dri/renderD129} \
  -serial unix:/mnt/work/ser.sock,server,nowait -monitor unix:/mnt/work/gui.sock,server,nowait -no-reboot \
  -drive if=pflash,format=raw,readonly=on,file=/usr/share/qemu/edk2-x86_64-code.fd \
  -cdrom /mnt/work/$ISO -boot d"
