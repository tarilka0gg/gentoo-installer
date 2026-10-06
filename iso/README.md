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

## Installing from the image, headless

`installer-cli --headless` (see the main README) was run inside a VM booted from the minimal image, onto a
blank virtio disk, against a local test store (an HTTP server with `Packages`, the live kernel and its modules;
a git server for the overlay; both on loopback, reached as `10.0.2.2`). The installed disk then booted by itself
(OVMF → Limine → kernel → OpenRC) to a login prompt with the hostname the installer set; logging in as the created
user gives fish with `micro`, `nano` absent, the btrfs subvolumes (`@`, `@home`, `@var`, `@log`) and `/boot` mounted
from `/etc/fstab`, and `doas` installed. Not covered: the desktop and GPU steps (the legacy path that installs
niri was not run), a real store with real kernels, real hardware.

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

## The stage3: downloaded by default, optionally shipped on the ISO

By default the image carries **no** stage3 (that was 240 MB of the disc): `prepare-rootfs.sh` writes `/etc/profile.d/installer-stage.sh` and
`/etc/fish/conf.d/20-installer-stage.fish`, which set `GENTOO_INSTALLER_STAGE3_URL` to the latest Simple Linux release's stage3
(`STAGE3_DEFAULT_URL=` changes it at build time) when nothing is set and no stage3 is on the medium. For a URL without a digest the installer
reads `<url>.sha512` (`sha512sum` format) next to it and verifies the download against that (`stage3::with_sidecar_digest`); the releases
publish that file. Checked in a VM: the image booted without a bundled stage, the install pulled the stage3 from GitHub, and the installed system had fish, eza,
micro and no nano. Sizes without it: minimal 851 MiB, GUI 1227 MiB.

An offline image: `STAGE_TARBALL=<file> assemble-iso.sh …` copies the tarball (and its `.sha512`) to `stage/` on the
medium — outside the squashfs, so it does not occupy RAM (checked: ~330 MB used with it on the disc).
The installer finds it at `/run/initramfs/live/stage` (`stage3::bundled`), verifies the digest and
unpacks it, so the stage3 step needs no network. Precedence: `GENTOO_INSTALLER_STAGE3_URL` >
the bundled stage > Gentoo's latest from the mirror. Sizes with it: minimal **746 MB**, GUI **1.3 GB**
(506 MB / 996 MB without). Verified in a VM: the file is visible read-only at that path and
`sha512sum -c` passes; the download-verify-unpack-user-gets-fish chain is covered by the real-target test.

## Firmware and rescue tools

Both images carry `linux-firmware` (Wi-Fi, GPU, Bluetooth, audio; the kernel's drivers are useless on
real hardware without it), `sof-firmware` and `intel-microcode`, and the usual rescue kit: `xfsprogs`,
`ntfs-3g`, `exfatprogs`, `f2fs-tools`, `cryptsetup`, `lvm2` (with its tools — the stage3 builds it
without), `mdadm`, `testdisk`, `ddrescue`, `smartmontools`, `nvme-cli`, `hdparm`, `usbutils`,
`dmidecode`, `lsof`, `htop`, `tmux`, `tcpdump`, `ethtool`, `strace`. The kernel fragment adds the modules
for them (`dm-crypt`, software RAID, XFS, F2FS, NTFS3, exFAT) and compressed-firmware loading.
Checked in a VM: the modules load and the tools are on `PATH`.

`linux-firmware` is 1.9 GB raw, so `assemble-iso.sh` leaves out what laptops and desktops do not use (server
NICs, SoC/phone blobs, old per-chip Wi-Fi versions; the two newest `iwlwifi` firmwares per chip stay),
which brings it to ~314 MB compressed. The microcode is not put into the initramfs (it is stored
uncompressed and the ISO carries the initramfs twice); the files stay in `/lib/firmware`.

Sizes with the firmware, the rescue kit, early microcode and the bundled stage: minimal **1.3 GB**, GUI **1.8 GB**.

## Booting on real hardware: the first laptop report, and the boot menu

The first real-hardware try stopped after Limine with `TSC_DEADLINE disabled ... please update microcode`
and `usb 2-9: device descriptor read/64, error -71`. Both are kernel *warnings*; the real problem was what
the images did around them, none of which a VM shows:

- `console=tty0 console=ttyS0,115200` makes the **last** one `/dev/console`. On a machine whose `ttyS0` is
  a phantom UART, dracut and init then talk to nothing and the screen freezes on the kernel's last
  warning. The default entry now has the screen only (`console=tty0`).
- `quiet` hid everything after those warnings, which turned "slow" into "looks hung". Gone (`loglevel=5`).
- `dracut --no-early-microcode` (my size saving) is exactly why the kernel complained about microcode.
  Early microcode is back: a 38 MB `GenuineIntel.bin` and 0.3 MB `AuthenticAMD.bin` in front of the
  initramfs (the ISO carries the initramfs twice, ~+80 MB). QEMU cannot show it taking effect
  (a hypervisor guest never loads microcode); `lsinitrd` shows the early CPIO in place.
- The serial getty used to respawn on a port that may not exist; `serial-getty` now idles unless
  the command line has `console=ttyS0`.

The Limine menu (5 s) has five entries, so a bad guess costs one reboot rather than one rebuild:
**live** (default), **safe graphics** (`nomodeset`, ends in the text installer), **USB workaround**
(`usbcore.old_scheme_first=1 usbcore.autosuspend=-1`, the usual cure for `device descriptor read/64,
error -71` when the boot stick itself is the device that fails), **verbose** (`loglevel=7 rd.debug`) and
**serial console**. In a VM only the default and the serial entry were actually booted (the menu itself and the
other entries' command lines were checked by eye); what the others do on real hardware is unknown. dracut's `rd.shell` is on,
so a missing live medium drops to a shell instead of hanging.

`test/run-gl-vm.sh` and `test/guest-shot.py` are the GL-VM test described below, kept in the repo.

## Added tools, Memtest86+, size

The rootfs also carries `gptfdisk`, `iw`, `wpa_supplicant`, `nmap`, `traceroute`, `eix`, `gentoolkit`, `screen` and `memtest86+`
(the picks from the official installcd package list that fit our use). They are emerged into both rootfs trees
(`USE="ncat nping"`), Python stays in the squashfs for `equery`, and `assemble-iso.sh` copies Memtest86+ to `boot/memtest/` and adds
two menu entries (`protocol: linux` for the BIOS image, `protocol: efi` for the UEFI one; both hash-pinned by `sign.sh`). `DEFAULT_ENTRY=6`
(or 7) makes the build boot straight into one, which is how both were tested. `/opt/rust-bin-*` and `/opt/zig-bin-*` are build-time
only and are excluded from the squashfs. Sizes: minimal 1091 MiB, gui 1467 MiB.

## Zen Browser and NVIDIA firmware are not on the medium

Zen (113 MB compressed) and `usr/lib/firmware/nvidia` (the GSP firmware nouveau needs on RTX 20xx and newer, 102 MB) are left out of the squashfs. The
GUI image has `get-zen` and a "Zen Browser" menu entry: the first start downloads the tarball from the project's GitHub release (`ZEN_URL` overrides it;
no checksum is published there, so it is HTTPS only) into `~/.local/share/zen` (RAM on the live system), opens a terminal for the progress when started from a
menu, then runs it. Checked in a VM: 111 MB downloaded, unpacked and started (`Mozilla Zen 1.23b`). Without the NVIDIA firmware the live GUI on a new NVIDIA card has no accelerated
nouveau and falls back to the software (cage) installer; the installed system uses the proprietary driver and is not affected. Sizes: minimal 747 MiB, GUI about 1000 MiB.

## ustan in the images

[ustan](https://github.com/tarilka0gg/ustan) (the Windows-style installer for .deb, AppImage, Flatpak and .exe) is in both images: the GUI image has
`ustan` and `ustan-gui` (with the menu entry), the minimal one only the `ustan` command. `add-ustan.sh <rootfs> gui|cli` emerges it into a rootfs from its
live ebuild, which it puts in a small overlay inside the rootfs (the ebuild comes from the ustan checkout next to this repository, `USTAN_EBUILD_DIR`
overrides). Rust and Zig stay build-time only (`zig-bin` is installed first so Portage does not compile Zig from source). `ustan-gui` registers itself
as the handler of .deb/.AppImage/.flatpak files on its first start (`ustan unregister` undoes it).

## Startup time

Profiled in QEMU/KVM (2 CPUs): from the kernel's start every OpenRC service is up in 2-4 s (EROFS image: `local` at 4.6 s), so the wait for the GUI was
not the system booting. `live-gui` slept a fixed **25 s** whenever niri stayed alive, and without a 3D GPU niri stays alive without an output, then 3 s more
to stop it before cage started (the installer window came ~31 s after the kernel started). It now looks at the DRM drivers: with a 3D-capable one
(`i915 xe amdgpu radeon nouveau nvidia virtio_gpu vmwgfx` and the ARM ones) it starts niri at once and polls for an output twice a second (up to 15 s);
with none after 2 s it lets udev settle, looks 1.5 s more and goes to cage: the window starts **~9 s** after the kernel instead of ~31 s (measured, twice).
The Limine menu waits 3 s instead of 5. A 3D virtual GPU (virgl) still takes ~14 s to reach niri because OpenRC sits ~10 s between sysinit and the boot
runlevel there; I did not find why and could not tell whether it also happens on real GPUs.

## What was cut from the image (0.2.7)

| What | Why it was there | Saved (ISO) |
|---|---|---|
| the rootfs's own `/boot` (initramfs 50 MB, microcode images 30 MB, memtest) | copies of what is already on the medium | ~100 MB of the root image |
| Intel microcode for `06-ad/ae` (Granite Rapids), `06-af` (Sierra Forest), `06-dd`, `06-b6` (Grand Ridge), `06-cf` (Emerald Rapids): 37 → 23 MB (`prune-microcode.sh`) | the early-microcode cpio is stored uncompressed and the initramfs is on the ISO twice | initramfs 50 → 36 MiB, twice |
| all of Python's `site-packages` but `portage`, `_emerge`, `gentoolkit`; the stdlib's `test`, `idlelib`, `tkinter`, `ensurepip`, `pydoc_data`; the python scripts that are not portage/gentoolkit tools | only `equery` needs Python; the rest (sphinx, babel, meson, pygments, docutils, ...) built things | ~20 MB |

`06-8f` (Sapphire Rapids, also the Xeon W workstations) is kept; `KEEP_MICROCODE=all` keeps all of it. A CPU whose file is gone still boots on the firmware's microcode.
The equery import set was found with `python -X importtime` over its subcommands. Checked: `equery`, `import portage, gentoolkit`, an empty `/boot` and the three images booting.

## The root image: EROFS with LZMA

Measured on the GUI tree (2.8 GiB after the exclusions), same files each time:

| Image | Size | Build | `local` started (VM, 2 or 8 CPUs) |
|---|---|---|---|
| squashfs, zstd-19, 1 MiB blocks | 859 MiB | ~2 min | 2.6 s after kernel start (with `rc_parallel`) |
| EROFS, zstd-19/22, 1 MiB clusters, fragments + dedupe | 828 MiB | 12-14 min | not measured |
| **EROFS, lzma-9, 1 MiB clusters, fragments + dedupe** | **740 MiB** | **~1 min** | **4.6 s** (6.1 s without `rc_parallel`) |

`make-erofs.sh` builds it (needs `sys-fs/erofs-utils`; from a directory, not a tar: mkfs.erofs 1.8.10 with a tar and `fragdedupe=inode` stored
almost nothing), `ROOT_IMAGE=squashfs` keeps the old one. The kernel needs `CONFIG_EROFS_FS` with LZMA (in `kernel-live.config`); dracut 103+ mounts either
type from the same `squashfs.img`. Extra CPUs do not help the boot (the reads are mostly sequential); `rc_parallel="YES"` does (about 1.5 s on the EROFS image).
Zstd levels above 19 change nothing at 1 MiB blocks, and squashfs with xz is 9 times slower to decompress (146 vs 1328 MB/s per core) for 6-8 % less.

## Compressing on another machine

The squashfs is the only heavy step (zstd 19: about 1 minute here, 6.5 on a 4-thread Westmere build server). `SQUASHFS_FROM=<file>` makes `assemble-iso.sh`
use a prebuilt one. Used for 0.2.5: `tar -C <rootfs> --anchored --no-wildcards --exclude-from=<squashfs-excludes.txt with `./` in front> --numeric-owner --xattrs -cf rootfs.tar .`
(also excluding `dev/console`, `dev/null` and fifos, which a rootless container cannot create), copied to the server together with `mksquashfs` and its libraries from the
builder chroot, then `sqfstar -comp zstd -Xcompression-level 19 -b 1M -processors 4 -p "dev d 755 0 0" -p "dev/console c 600 0 0 5 1" -p "dev/null c 666 0 0 1 3" out.sqsh < rootfs.tar`
(`sqfstar` is `mksquashfs` under another name and reads the ownership from the tar). The result booted under Secure Boot like the locally built one.

## Ventoy, and CPUs older than Haswell

**Ventoy** ignores `limine.conf`; `assemble-iso.sh` also writes `boot/grub/grub.cfg` with the same entries. Checked with Ventoy
1.1.17 on a loop-backed disk image in QEMU: *Boot in grub2 mode* works on BIOS and UEFI, *normal mode* on UEFI; on BIOS normal
mode Ventoy chain-loads the ISO's Limine boot sector, which stops with "Could not determine boot drive". (Ventoy under SeaBIOS needs
the stick attached as a SATA/IDE disk in QEMU, not as `usb-storage`.)

**Older CPUs:** a binary linked on a machine whose libc is built with `-march=native` gets an "x86-64-v3 needed" ELF note and glibc
refuses to run it elsewhere (`CPU ISA level is lower than required`). `prepare-rootfs.sh` runs `strip-isa-note.sh` on the installer
binaries. Checked in QEMU with Westmere, Sandy Bridge and `qemu64` CPUs, BIOS and UEFI, 1-4 CPUs and 1-3 GB of RAM. Any other
binary added to the image from the build machine needs the same treatment (the stage3's own packages are generic).

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
- Secure Boot: images can be signed (see `secureboot/README.md`); the installed system is not set up for it yet. Releases carry `SHA256SUMS` and an ssh signature.
