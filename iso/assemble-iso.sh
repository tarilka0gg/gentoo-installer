#!/bin/bash
# assemble-iso.sh <work-dir> <out.iso>   (ROOTFS=live-root-gui selects the GUI tree)
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

cp "$W/vmlinuz-live" "$ISO/boot/vmlinuz"
cp "$W/$ROOTFS/boot/initramfs-live.img" "$ISO/boot/initramfs.img"
chmod 644 "$ISO/boot/"*
cp /usr/share/limine/limine-bios-cd.bin /usr/share/limine/limine-bios.sys "$ISO/boot/limine/"
cp /usr/share/limine/BOOTX64.EFI "$ISO/EFI/BOOT/"

cat > "$ISO/limine.conf" <<CONF
timeout: 3
serial: yes

/Gentoo installer (live)
    protocol: linux
    kernel_path: boot():/boot/vmlinuz
    module_path: boot():/boot/initramfs.img
    cmdline: root=live:CDLABEL=$LABEL rd.live.image rd.live.overlay.overlayfs=1 rd.live.dir=LiveOS rd.live.squashimg=squashfs.img console=tty0 console=ttyS0,115200 quiet ${EXTRA_CMDLINE:-}
CONF
cp "$ISO/limine.conf" "$ISO/boot/limine/limine.conf"

# UEFI: El Torito boots a small FAT image holding Limine; it finds boot() there, so the
# kernel and initramfs are copied in too (~30 MB — cheaper than a second config dialect).
EFI=$W/efiboot.img
# Sized to what goes in (+8 MB slack); FAT32 with 512-byte clusters needs >= ~33 MB to be valid.
need=$(( $(stat -c %s "$ISO/boot/vmlinuz" "$ISO/boot/initramfs.img" "$ISO/EFI/BOOT/BOOTX64.EFI" | paste -sd+ | bc) / 1048576 + 8 ))
[ "$need" -lt 36 ] && need=36
rm -f "$EFI"; truncate -s ${need}M "$EFI"; mkfs.vfat -F 32 -s 1 -n EFIBOOT "$EFI" >/dev/null
mmd -i "$EFI" ::/EFI ::/EFI/BOOT ::/boot
mcopy -i "$EFI" "$ISO/EFI/BOOT/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$EFI" "$ISO/limine.conf" ::/limine.conf
mcopy -i "$EFI" "$ISO/boot/vmlinuz" "$ISO/boot/initramfs.img" ::/boot/
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
usr/include
usr/share/locale
usr/share/i18n
usr/share/sgml
usr/share/cmake
usr/share/gcc-data
usr/share/binutils-data
usr/libexec/gcc
usr/lib/binutils
usr/lib/python3.14
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

# squashfs + xorriso inside the builder chroot
cat > "$W/inner.sh" <<INNER
set -e
mksquashfs /mnt/live-root /mnt/work/isoroot/LiveOS/squashfs.img -comp zstd -Xcompression-level 12 -b 256K -noappend -no-progress -ef /mnt/work/squashfs-excludes.txt
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
