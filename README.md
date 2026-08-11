# Gentoo Installer

Installer for a Gentoo-based distro built on top of a custom Portage store
(binhost + overlay, see `../portage-store`). Two separate ISOs, two separate
frontends over one shared core:

- **minimal ISO** — no GUI, no WM/DE at all. `installer-cli` (ratatui TUI).
- **main ISO** — niri + a shell package preinstalled on the target,
  GTK4 + libadwaita installer (`installer-gui`).

## Base system

- **Stage3**: official Gentoo `amd64-openrc` autobuild — multilib, non-hardened,
  OpenRC init. Not building a custom stage3 via catalyst; the distro's identity
  lives in the overlay/binhost and in the installer itself, not the base tarball.
- **Kernel**: no on-device compilation. Precompiled binary kernel packages per
  hardware profile, published to the store ahead of time by a separate build
  script. The installer detects CPU vendor / GPU vendor / RAM / laptop-vs-desktop
  and matches against `sys-kernel/<name>-bin-<cpu>-<gpu>`, falling back to
  `-generic` (`installer-core::hardware`, `installer-core::kernel`). Detection is
  hybrid — auto by default, manually overridable.
- **Bootloader**: Limine only. No GRUB.
- **Partitioning**: automatic, no manual step.
  - ESP: 512 MiB, vfat (required for Limine on UEFI)
  - swap: always created, sized 1:1 with RAM, clamped to 8–96 GiB
    (`installer-core::partition::swap_size_gib`) — exists mainly as an
    overflow buffer for parallel (`-j`) compiles, not for hibernation
  - root: btrfs by default with subvolumes `@` / `@home` / `@var` / `@log`;
    ext4 (single partition, no subvolumes) available as an alternative.
    No LUKS — considered unnecessary overhead; users who want disk encryption
    set it up manually.
- **Network**: iwd over D-Bus, not NetworkManager (too heavy for a live image).
  Ethernet is autodetected via link state so the wifi step is skippable.

## Workspace layout

```
installer-core/   lib crate — all real logic, no UI code
  hardware.rs      CPU/GPU/RAM/chassis detection -> Profile
  kernel.rs        Profile -> matching store atom
  partition.rs     layout planning + swap sizing + apply()
  stage3.rs        resolve/download/unpack official stage3
  bootloader.rs     Limine config generation + install
  network.rs       iwd client (zbus) + ethernet link check
  store.rs         wires target root to the distro's overlay/binhost

installer-cli/    ratatui TUI binary — minimal ISO only
installer-gui/    gtk4-rs + libadwaita binary — main ISO only
```

Both frontends call into `installer-core` only; no logic is duplicated
between them.

## Status

Scaffolding stage: workspace builds (`cargo check` clean across all three
crates, `cargo test -p installer-core` passing for swap-sizing), module
boundaries and function signatures are in place, most bodies are `todo!()`
pending implementation. GPU vendor detection (`hardware::detect_gpu_vendor`)
still needs `lspci` parsing. Package-set selection (what gets installed
beyond base system — DE/WM flavors, etc.) is intentionally not designed yet.
