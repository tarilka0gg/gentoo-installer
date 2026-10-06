#!/bin/bash
# assemble-iso.sh <work-dir> <out.iso>   (ROOTFS=live-root-gui selects the GUI tree;
#                                         STAGE_TARBALL=<file> ships a custom stage3 on the medium)
# work-dir has: live-root/ (prepared rootfs incl. /boot/initramfs-live.img), vmlinuz-live,
# builder/ (a chroot that has mksquashfs + xorriso). Run as root.
# The squashfs/xorriso steps run inside the builder chroot; mounts live in a private
# namespace, so nothing outlives the script.
set -euo pipefail
W=$(readlink -f "${1:?work dir}"); OUT=$(readlink -f "${2:?out.iso}")
LABEL=GENTOO_LIVE
ROOTFS=${ROOTFS:-live-root}
ISO=$W/isoroot
rm -rf "$ISO"; mkdir -p "$ISO"/{boot/limine,LiveOS,EFI/BOOT}

# Optional: ship a stage3 on the medium (outside the squashfs, so it is not loaded into RAM).
# The installer finds it at /run/initramfs/live/stage (stage3::bundled).
if [ -n "${STAGE_TARBALL:-}" ]; then
    mkdir -p "$ISO/stage"
    cp "$STAGE_TARBALL" "$ISO/stage/"
    [ -e "$STAGE_TARBALL.sha512" ] && cp "$STAGE_TARBALL.sha512" "$ISO/stage/"
fi

# The rootfs was built in chroots that borrowed the build host's /etc/resolv.conf. Do not ship it:
# it names the host's resolver (and, on the author's machine, a private Tailscale network).
# dhcpcd writes the real one at boot.
: > "$W/$ROOTFS/etc/resolv.conf"

cp "$W/vmlinuz-live" "$ISO/boot/vmlinuz"
# Memtest86+ (installed in the rootfs) boots straight from the menu, BIOS or UEFI.
MEMTEST=
if [ -f "$W/$ROOTFS/boot/memtest86plus/memtest64.bios" ] && [ -f "$W/$ROOTFS/boot/memtest86plus/memtest.efi64" ]; then
    mkdir -p "$ISO/boot/memtest"
    cp "$W/$ROOTFS/boot/memtest86plus/memtest64.bios" "$W/$ROOTFS/boot/memtest86plus/memtest.efi64" "$ISO/boot/memtest/"
    MEMTEST=1
fi
cp "$W/$ROOTFS/boot/initramfs-live.img" "$ISO/boot/initramfs.img"
chmod 644 "$ISO/boot/"*
cp /usr/share/limine/limine-bios-cd.bin /usr/share/limine/limine-bios.sys "$ISO/boot/limine/"
cp /usr/share/limine/BOOTX64.EFI "$ISO/EFI/BOOT/"

# Real hardware first. Three lessons from the first laptop test:
#  * `console=ttyS0` last made the serial port /dev/console; a laptop with a phantom UART then showed
#    the kernel's last warning and nothing else (dracut and init were talking to the void).
#    So the default has the screen only; serial is its own entry (and EXTRA_CMDLINE for tests).
#  * `quiet` hid every message after the first warnings, which turned "slow" into "hung".
#  * Limine's own `serial: yes` is only for the serial entry's benefit and is off by default.
BASE="root=live:CDLABEL=$LABEL rd.live.image rd.live.dir=LiveOS rd.live.squashimg=squashfs.img rd.shell"
cat > "$ISO/limine.conf" <<CONF
timeout: 5
default_entry: ${DEFAULT_ENTRY:-1}

/Simple Linux (live)
    protocol: linux
    kernel_path: boot():/boot/vmlinuz
    module_path: boot():/boot/initramfs.img
    cmdline: $BASE console=tty0 loglevel=5 ${EXTRA_CMDLINE:-}

/Simple Linux - safe graphics (nomodeset, text installer)
    protocol: linux
    kernel_path: boot():/boot/vmlinuz
    module_path: boot():/boot/initramfs.img
    cmdline: $BASE console=tty0 loglevel=5 nomodeset ${EXTRA_CMDLINE:-}

/Simple Linux - USB workaround (legacy enumeration, no autosuspend)
    protocol: linux
    kernel_path: boot():/boot/vmlinuz
    module_path: boot():/boot/initramfs.img
    cmdline: $BASE console=tty0 loglevel=5 usbcore.old_scheme_first=1 usbcore.autosuspend=-1 ${EXTRA_CMDLINE:-}

/Simple Linux - verbose (every kernel and initramfs message, drops to a shell on failure)
    protocol: linux
    kernel_path: boot():/boot/vmlinuz
    module_path: boot():/boot/initramfs.img
    cmdline: $BASE console=tty0 loglevel=7 rd.debug ${EXTRA_CMDLINE:-}

/Simple Linux - serial console (ttyS0, 115200)
    protocol: linux
    kernel_path: boot():/boot/vmlinuz
    module_path: boot():/boot/initramfs.img
    cmdline: $BASE console=tty0 console=ttyS0,115200 loglevel=5 ${EXTRA_CMDLINE:-}
CONF
if [ -n "$MEMTEST" ]; then
cat >> "$ISO/limine.conf" <<CONF

/Memory test - Memtest86+ (BIOS)
    protocol: linux
    kernel_path: boot():/boot/memtest/memtest64.bios

/Memory test - Memtest86+ (UEFI)
    protocol: efi
    image_path: boot():/boot/memtest/memtest.efi64
CONF
fi
# Optional Secure Boot: SECUREBOOT_KEYS=<dir from secureboot/make-keys.sh> signs Limine and pins the config,
# kernel and initramfs by hash (see secureboot/sign.sh); db.cer goes on the medium for enrolling.
if [ -n "${SECUREBOOT_KEYS:-}" ]; then
    ROOT=$ISO "$(dirname "$(readlink -f "$0")")/secureboot/sign.sh" "$SECUREBOOT_KEYS" "$ISO/limine.conf" "$ISO/EFI/BOOT/BOOTX64.EFI"
    mkdir -p "$ISO/secureboot"; cp "$SECUREBOOT_KEYS/db.cer" "$ISO/secureboot/simple-linux-db.cer"
fi
cp "$ISO/limine.conf" "$ISO/boot/limine/limine.conf"

# Ventoy ignores limine.conf: it reads GRUB-style menus ("Boot in grub2 mode", and the normal mode for Linux ISOs), loads
# the kernel itself, and injects the hook that makes the ISO visible as /dev/mapper/ventoy. Same entries as Limine's.
mkdir -p "$ISO/boot/grub"
cat > "$ISO/boot/grub/grub.cfg" <<GRUBCFG
set timeout=5
set default=0

menuentry "Simple Linux (live)" {
    linux /boot/vmlinuz $BASE console=tty0 loglevel=5 ${EXTRA_CMDLINE:-}
    initrd /boot/initramfs.img
}
menuentry "Simple Linux - safe graphics (nomodeset, text installer)" {
    linux /boot/vmlinuz $BASE console=tty0 loglevel=5 nomodeset ${EXTRA_CMDLINE:-}
    initrd /boot/initramfs.img
}
menuentry "Simple Linux - USB workaround (legacy enumeration, no autosuspend)" {
    linux /boot/vmlinuz $BASE console=tty0 loglevel=5 usbcore.old_scheme_first=1 usbcore.autosuspend=-1 ${EXTRA_CMDLINE:-}
    initrd /boot/initramfs.img
}
menuentry "Simple Linux - verbose (every kernel and initramfs message)" {
    linux /boot/vmlinuz $BASE console=tty0 loglevel=7 rd.debug ${EXTRA_CMDLINE:-}
    initrd /boot/initramfs.img
}
GRUBCFG

# UEFI: El Torito boots a small FAT image holding Limine; it finds boot() there, so the
# kernel and initramfs are copied in too (~30 MB - cheaper than a second config dialect).
EFI=$W/efiboot.img
# Sized to what goes in (+8 MB slack); FAT32 with 512-byte clusters needs >= ~33 MB to be valid.
need=$(( $(stat -c %s "$ISO/boot/vmlinuz" "$ISO/boot/initramfs.img" "$ISO/EFI/BOOT/BOOTX64.EFI" | paste -sd+ | bc) / 1048576 + 12 ))
[ "$need" -lt 36 ] && need=36
rm -f "$EFI"; truncate -s ${need}M "$EFI"; mkfs.vfat -F 32 -s 1 -n EFIBOOT "$EFI" >/dev/null
mmd -i "$EFI" ::/EFI ::/EFI/BOOT ::/boot
mcopy -i "$EFI" "$ISO/EFI/BOOT/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$EFI" "$ISO/limine.conf" ::/limine.conf
mcopy -i "$EFI" "$ISO/boot/vmlinuz" "$ISO/boot/initramfs.img" ::/boot/
if [ -n "$MEMTEST" ]; then mmd -i "$EFI" ::/boot/memtest; mcopy -i "$EFI" "$ISO/boot/memtest/"* ::/boot/memtest/; fi
cp "$EFI" "$ISO/boot/efiboot.img"

# Build-time-only weight: the Portage tree, compilers' data, headers and docs. The live
# system never compiles anything (the installer chroots into the *target*).
# Mesa's llvmpipe/radeonsi load libLLVM at run time, so a rootfs with the GUI keeps that one
# library (found the hard way: niri had no outputs and the screen stayed black).
if [ -e "$W/$ROOTFS/usr/local/bin/installer-gui" ]; then
    LLVM_EXCLUDES="usr/lib/llvm/22/bin
usr/lib/llvm/22/include
usr/lib/llvm/22/share
usr/lib/llvm/22/libexec
usr/lib/llvm/22/lib64/libclang-cpp.so.22.1
usr/lib/llvm/22/lib64/libclang.so.22.1.8"
else
    LLVM_EXCLUDES="usr/lib/llvm"
fi
# Toolchains installed under /opt only to build things (Rust, Zig): the live system compiles nothing.
OPT_EXCLUDES=$(cd "$W/$ROOTFS" && ls -d opt/rust-bin-* opt/zig-bin-* 2>/dev/null || true)
cat > "$W/squashfs-excludes.txt" <<EXCL
var/db/repos/gentoo
var/db/repos/guru
usr/lib/rust
usr/lib/go
usr/lib/go-bootstrap
usr/lib/clang
usr/lib/grub
var/tmp/portage
var/cache/distfiles
var/cache/binpkgs
$LLVM_EXCLUDES
$OPT_EXCLUDES
usr/include
usr/share/locale
usr/share/i18n
usr/share/sgml
usr/share/cmake
usr/share/gcc-data
usr/share/binutils-data
usr/libexec/gcc
usr/lib/binutils
usr/bin/sway
usr/bin/swaymsg
usr/bin/swaynag
usr/bin/swaybar
usr/share/sway
usr/share/man
usr/share/doc
usr/share/info
usr/share/gtk-doc
usr/lib/python3.14/test
EXCL

# linux-firmware is ~1.9 GB raw. Keep what laptops/desktops need (Wi-Fi, GPU, Bluetooth, audio,
# Ethernet); leave out server NICs, SoC/phone firmware, and old per-chip Wi-Fi versions.
# Computed from the rootfs, so a missing firmware dir just adds nothing.
python3 - "$W/$ROOTFS" >> "$W/squashfs-excludes.txt" <<'FWPY'
import glob, os, re, sys, collections
root = sys.argv[1]
fw = os.path.join(root, "usr/lib/firmware")
if os.path.isdir(fw):
    drop = """qcom netronome mellanox mrvl qed dpaa2 liquidio cxgb4 bnx2x bnx2 myri10ge sfc e100 tigon
    ql2xxx ql2400 ql2500 ti-keystone amphion imx nxp arm rockchip meson powervr vpu airoha
    mediatek/mt8* qat_* intel/ipu intel/vsc intel/ice intel/qat intel/vpu intel/catpt intel/avs""".split()
    for pat in drop:
        for p in glob.glob(os.path.join(fw, pat)):
            print("usr/lib/firmware/" + os.path.relpath(p, fw))
    # Wi-Fi: per chip the kernel asks for the newest API versions; keep the two newest.
    d = os.path.join(fw, "intel/iwlwifi")
    groups = collections.defaultdict(list)
    for f in os.listdir(d) if os.path.isdir(d) else []:
        m = re.match(r"(iwlwifi-.+?)-(\d+)\.(ucode|pnvm)$", f)
        if m:
            groups[(m.group(1), m.group(3))].append((int(m.group(2)), f))
    for v in groups.values():
        v.sort(reverse=True)
        for _, f in v[2:]:
            print("usr/lib/firmware/intel/iwlwifi/" + f)
FWPY

# squashfs + xorriso inside the builder chroot
cat > "$W/inner.sh" <<INNER
set -e
mksquashfs /mnt/live-root /mnt/work/isoroot/LiveOS/squashfs.img -comp zstd -Xcompression-level 19 -b 1M -noappend -no-progress -ef /mnt/work/squashfs-excludes.txt
xorriso -as mkisofs -iso-level 3 -full-iso9660-filenames -volid $LABEL -R -J \
  -b boot/limine/limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table \
  --efi-boot boot/efiboot.img -efi-boot-part --efi-boot-image --protective-msdos-label \
  -o /mnt/work/out.iso /mnt/work/isoroot
INNER
cat > "$W/outer.sh" <<OUTER
set -e
B=$W/builder
mkdir -p \$B/mnt/live-root \$B/mnt/work
mount --bind /proc \$B/proc; mount --rbind /dev \$B/dev
mount --bind $W/$ROOTFS \$B/mnt/live-root
mount --bind $W \$B/mnt/work
chroot \$B nice -n 10 bash /mnt/work/inner.sh
OUTER
unshare --mount --propagation private bash "$W/outer.sh"

limine bios-install "$W/out.iso" >/dev/null
mv "$W/out.iso" "$OUT"
ls -lh "$OUT"
