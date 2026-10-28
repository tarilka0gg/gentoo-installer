# Changelog

All notable changes to gentoo-installer, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/), with
`## [version] - date` headings.

Releases of the installer are the Simple Linux images; what changed in each is in the
[Simple Linux changelog](https://github.com/tarilka0gg/simple-linux/blob/main/CHANGELOG.md). This file lists the code changes that have
no release of their own yet.

## [Unreleased]

### Changed
- Live image: BusyBox 1.36.1 (static, musl, built with `zig cc`, 242 KiB; `iso/busybox-userland.sh`, `iso/busybox.config`) takes the place of
  the 83 GNU coreutils programs it has an applet for; the other 19 stay GNU and the installed system is untouched. Run after dracut, so the
  initramfs keeps the GNU tools. `BUSYBOX_USERLAND=0` in `refresh-rootfs.sh` skips it.
- Live image: firmware that the live kernel's modules never name is left out of the squashfs for the GPU, Realtek, Atheros and MediaTek Wi-Fi
  directories (`iso/firmware-excludes.py`; about 27 MB off each ISO). Intel Wi-Fi is not trimmed because iwlwifi does not list all the files it
  loads. `FIRMWARE_TRIM=0` in `assemble-iso.sh` skips it.
- The ISO's homectl is `app-admin/homectl-bin` 0.20.0 (was 0.19.3): live webcam video, Wake up, the performance profile and the regrouped
  app tabs. The ebuild and Manifest are built from the release's assets, and their checksums match the release's `SHA256SUMS`.

### Added
- The Software page shows each program's sentence and, for many, a screenshot that opens full-size on a click.
- `GENTOO_INSTALLER_GUI_PAGE=desktop|network` (besides the existing pages) opens the wizard on that page, for looking at it.
- `iso/check-upstream.py` and a weekly workflow: what Simple Linux pins (kernel, stage3, the binary host's tree snapshot, the overlay's
  ebuilds) against what upstream has now; `profiles/overlay/bump.py` moves an overlay package to a new release.
- The ISO ships `/etc/simple-linux/locale.toml`, the shared language setting.

## [0.1.0] - 2026-10-07

### Added
- First tagged version, packaged for GURU: a text and a GTK4/libadwaita installer over one Rust core, a binary install with a kernel picked for
  the hardware, and the scripts that build the live images.
