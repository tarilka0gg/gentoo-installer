# Changelog

All notable changes to gentoo-installer, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/), with
`## [version] - date` headings.

Releases of the installer are the Simple Linux images; what changed in each is in the
[Simple Linux changelog](https://github.com/tarilka0gg/simple-linux/blob/main/CHANGELOG.md). This file lists the code changes that have
no release of their own yet.

## [Unreleased]

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
