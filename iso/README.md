# Live ISO (minimal, TUI)

Boots on BIOS and UEFI, autologins as root and starts `installer-cli`. About 750 MB.
Tested in QEMU (KVM) with SeaBIOS and OVMF: Limine → kernel → dracut (`dmsquash-live`,
OverlayFS) → OpenRC → the installer's TUI on tty1 and on the serial console.

## What is in it

| Part | Where it comes from |
|---|---|
| Kernel 7.1.8 + modules | `x86_64_defconfig` + `kernel-live.config` (squashfs/overlay/iso9660/loop/SATA/NVMe/USB/virtio/btrfs built in; Wi-Fi, Ethernet, audio, GPU as modules) |
| Root filesystem | a stage3 (openrc) plus what `installer-cli` calls: parted, git, btrfs-progs, dosfstools, os-prober, cpuid2cpuflags, limine, pciutils, iwd |
| Initramfs | dracut with `dmsquash-live`, run inside the root filesystem |
| Image | `mksquashfs` (zstd) + `xorriso`, Limine for BIOS (El Torito) and UEFI (FAT image) |

The Portage tree, LLVM, headers and docs are left out of the squashfs
(`squashfs-excludes.txt` is written by `assemble-iso.sh`): the live system never compiles;
the installer works in a chroot of the *target*.

## Building

Nothing touches the host: kernel sources are copied (not built in `/usr/src`), and the
rootfs and tool chroots are copies of a stage3 that are entered in a private mount
namespace (`unshare --mount --propagation private`).

1. **Kernel.** Copy the sources without build artifacts (`rsync -a` excluding `*.o`, `*.ko`, …
   but **not** `vmlinux*.S/.h` or `*.bc` — those are real sources), then
   `make x86_64_defconfig`, merge `kernel-live.config` with `scripts/kconfig/merge_config.sh -m`,
   `make olddefconfig && make -j`, `make modules_install INSTALL_MOD_PATH=<work>/modroot`.
   Copy `bzImage` to `<work>/vmlinuz-live`.
2. **Tool chroot** (`<work>/builder`): a stage3 copy with `libisoburn`, `squashfs-tools`
   (`USE="lzo xz zstd"`), `mtools`, `dosfstools`.
3. **Root filesystem** (`<work>/live-root`): a stage3 copy; emerge
   `parted git btrfs-progs dosfstools os-prober cpuid2cpuflags limine pciutils iwd dracut`
   with `LC_ALL=C.UTF-8` (Sphinx documentation builds fail on a stage3 without generated
   locales) and `sys-boot/limine ~amd64`.
4. **Prepare and initramfs**, as root in a private mount namespace with `/proc /sys /dev` bound in:
   copy the modules to `live-root/lib/modules/`, run `prepare-rootfs.sh live-root
   target/release/installer-cli`, `depmod`, then
   `dracut --no-hostonly --kver <ver> --add dmsquash-live --compress zstd /boot/initramfs-live.img`.
5. **Assemble:** `assemble-iso.sh <work> out.iso`.

Boot parameters (in `limine.conf`): `root=live:CDLABEL=GENTOO_LIVE rd.live.image
rd.live.overlay.overlayfs=1` — the kernel has device-mapper but no snapshot target, so the
overlay is OverlayFS.

## Testing

```
qemu-system-x86_64 -machine q35 -enable-kvm -m 2048 -display none -serial stdio \
    -cdrom out.iso -boot d                      # BIOS
    ... -drive if=pflash,format=raw,readonly=on,file=/usr/share/qemu/edk2-x86_64-code.fd   # UEFI
```

## Not done

- The GUI ("main") ISO: niri, GTK4/libadwaita and `installer-gui` are not in the image yet.
- Nothing runs a real install from the ISO: the store has no kernels to install (see the
  main README), so the TUI has been started but not driven end to end.
- No checksums, no signing, no Secure Boot.
