# Live ISOs (minimal TUI, and GUI)

Boots on BIOS and UEFI, autologins as root and starts `installer-cli`. 506 MB.
Tested in QEMU (KVM) with SeaBIOS and OVMF: Limine → kernel → dracut (`dmsquash-live`,
OverlayFS) → OpenRC → the installer's TUI on tty1 and on the serial console.

## What is in it

| Part | Where it comes from |
|---|---|
| Kernel 7.1.8 + modules | `x86_64_defconfig` + `kernel-live.config` (squashfs/overlay/iso9660/loop/SATA/NVMe/USB/virtio/btrfs built in; Wi-Fi, Ethernet, audio, GPU as modules) |
| Root filesystem | a stage3 (openrc) plus what `installer-cli` calls: parted, git, btrfs-progs, dosfstools, os-prober, cpuid2cpuflags, limine, pciutils, iwd |
| Initramfs | dracut with `dmsquash-live`, run inside the root filesystem |
| Image | `mksquashfs` (zstd) + `xorriso`, Limine for BIOS (El Torito) and UEFI (FAT image) |

The Portage tree, LLVM, compilers, Python, headers, translations and docs are left out of the squashfs
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

## Size

Measured with `ls -l` on the built images: minimal 755 → **506 MB**, GUI 1.1 GB → **996 MB** (with the apps below) after
(1) sizing the UEFI FAT image to its content instead of a fixed 96 MB, and (2) leaving out what a
live system never uses: `usr/share/{locale,i18n,sgml,cmake,gcc-data,binutils-data}`, GCC's
`libexec`, binutils, Python, the Go toolchain (micro is written in Go, so building it pulls
~500 MB of Go into the rootfs), clang's resource dir, GRUB's files (the installer uses Limine)
and (GUI) Sway, which the first emerge pulled in. All three boot
modes were re-tested after trimming (BIOS and UEFI for minimal, UEFI for GUI).
For comparison, the official Gentoo minimal install CD is 975.8 MiB and its LiveGUI 4.0 GiB
(distfiles.gentoo.org, 2026-09-13 builds).

## GUI ISO

Same kernel and boot path, `ROOTFS=live-root-gui ./assemble-iso.sh <work> out.iso`. The rootfs
adds GTK4, libadwaita, Mesa (with LLVM), seatd, **niri, Noctalia, xwayland-satellite and
ghostty** — the compositor and shell from `gentoo-wm-configs`, with that repo's `niri/config.kdl`
and `noctalia/config.toml` copied unchanged into root's home. `prepare-rootfs.sh <root> <cli>
<gui> <wm-configs-dir>` appends two live-only blocks to the *copy* of the niri config (start
`installer-gui` as a floating window, so Noctalia's bar stays visible). tty1 starts `dbus-run-session -- niri --session` (stderr
in `/var/log/niri-session.log`); the serial console gets the TUI. 996 MB.

**niri needs hardware-accelerated graphics.** It skips software EGL renderers
(`software EGL renderers are skipped` in the log), so with no GPU driver it has no outputs. A
watchdog stops it after 25 s in that case and starts the text installer instead of leaving a
black screen. Verified in QEMU with a plain `virtio-vga`, which is exactly that situation.

The graphical path **was** verified in a VM with a 3D virtual GPU: QEMU built with
`USE="opengl virgl"` (in a stage3 chroot, x86_64 target only; the host QEMU has no GL display) and

    qemu-system-x86_64 -machine q35 -enable-kvm -m 4096 -device virtio-vga-gl \
        -display egl-headless,rendernode=/dev/dri/renderD129 -cdrom out.iso ...

niri then gets an output (`Virtual-1`, 1280x800), Noctalia starts and `installer-gui` opens full
window on its Welcome page (the bar on top). `screendump` does not work with GL scanout (`Error: no surface`);
take the picture inside the guest instead: `niri msg action screenshot-screen --write-to-disk true`
from the `live.debug` serial shell, and pull the PNG out as base64. Real hardware has not been tried.

Two first-run prompts would otherwise land on top of the installer, so the image pre-empts both:
Noctalia's setup wizard (`~/.local/state/noctalia/.setup-complete`) and gnome-keyring's "Choose
password for new keyring" (a plain, empty-password `Default_keyring` in `~/.local/share/keyrings`;
nothing secret lives in a live session). Both checked in the GL VM: only the installer window and
the bar remain.

Add `live.debug` to the kernel command line (`EXTRA_CMDLINE=live.debug` when assembling) to get a
plain shell on the serial console instead of the installer.

## Shipping the custom stage on the ISO

`STAGE_TARBALL=<file> assemble-iso.sh …` copies the tarball (and its `.sha512`) to `stage/` on the
medium — outside the squashfs, so it does not occupy RAM (checked: ~330 MB used with it on the disc).
The installer finds it at `/run/initramfs/live/stage` (`stage3::bundled`), verifies the digest and
unpacks it, so the stage3 step needs no network. Precedence: `GENTOO_INSTALLER_STAGE3_URL` >
the bundled stage > Gentoo's latest from the mirror. Sizes with it: minimal **746 MB**, GUI **1.3 GB**
(506 MB / 996 MB without). Verified in a VM: the file is visible read-only at that path and
`sha512sum -c` passes; the download-verify-unpack-user-gets-fish chain is covered by the real-target test.

## Shell and tools

Both images use **fish** as root's shell, with the author's aliases in `/etc/fish/conf.d/10-house.fish`
(`ls`/`ll`/`lt` → `eza`, `nano` → `micro`, `du` → `dust`, `ping` → `gping`, `EDITOR=micro`) and no
`nano`. The installer is started from `~/.config/fish/config.fish` (same logic as `.bash_profile`,
fish syntax). `micro` and `gping` are `~amd64` (`micro` from `gentoo`, `gping` from GURU).

## Desktop programs (GUI image)

Zen Browser (`zen-bin`, GURU), Thunar with gvfs and tumbler, GParted, PipeWire + WirePlumber
(Noctalia's volume widget talks to PipeWire; the session starts it), xdg-desktop-portal(-gtk),
`wl-clipboard`, `btop`, `imv`. Wi-Fi is Noctalia's network widget over `iwd`, which the image already
runs. GTK3 apps need `X`, so `gtkmm`/`cairomm`/`gtk+`/`cairo` are built with it (and run through
xwayland-satellite). Checked in the GL VM: the installer, Thunar and Zen open, PipeWire has clients
and the bar shows the volume control.

## Personal profile (optional)

`make-profile.py <dir>` (run as the user, not root) reads the current user's niri and Noctalia
setup and writes a copy that is safe to boot elsewhere; pass that directory as the fifth argument
of `prepare-rootfs.sh`. Kept: colours, layout, animations, key bindings, input, bar and dock
layout, theme and the community palettes it refers to. Dropped: start-up services and scripts,
monitor blocks, the iGPU render-device pin, per-app window rules, the home address, wallpaper
paths, per-monitor lock-screen widgets, and history/usage/clipboard data. The output is derived from a home directory, so keep it
out of git; the script itself contains no personal data.

## Bugs this found in the kernel config

Both are invisible with serial-only testing and were only noticed from screenshots:

- `CONFIG_FB` was missing, so `olddefconfig` silently dropped `FRAMEBUFFER_CONSOLE`: on any UEFI
  machine tty1 stayed black. Fixed in `kernel-live.config` (`FB`, `FB_EFI`, `DRM_FBDEV_EMULATION`,
  `FRAMEBUFFER_CONSOLE`); the TUI is now visible on a UEFI VM.
- Mesa loads `libLLVM.so` at run time. Excluding `usr/lib/llvm` from the squashfs (fine for the
  minimal ISO) broke the GUI ISO; `assemble-iso.sh` now keeps only that library when the rootfs
  contains the GUI.

## Not done
- The TUI's network screen says Wi-Fi (iwd) is "not wired into this screen yet": on a machine without Ethernet the installer cannot get online from the TUI.
- Nothing runs a real install from the ISO: the store has no kernels to install (see the
  main README), so the TUI has been started but not driven end to end.
- No checksums, no signing, no Secure Boot.
