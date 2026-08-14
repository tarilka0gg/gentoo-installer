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
- **Kernel**: no on-device compilation. Precompiled binary kernel packages, built
  ahead of time by `kernel-configs/matrix-build.sh` (real script, `../kernel-releases/`)
  across a 3-axis matrix — `<cpu>-<gpu>-<platform>` — e.g.
  `intel-raptorlake-nvidia-laptop`. EC/WMI vendor and modem support are *not* axes:
  every build always includes all vendor EC/modem drivers as modules (`ec/all` +
  `modem/all`), since `--skip-modules` means they never affected the bzImage anyway —
  splitting on them just produced identical kernels under different names, so that was
  dropped. `server` was also dropped from the platform axis (out of scope for this
  distro). 378 possible cpu×gpu×platform combinations exist (18 CPU codenames × 7 GPU ×
  3 platform), 304 are actually built by popularity (`gen-popular-targets.py`'s scoring),
  so `installer-core::kernel::resolve` degrades through `hardware::Profile::candidates()`
  (generalize platform → cpu → gpu) rather than requiring an exact match. Detection
  (`installer-core::hardware`) is real CPU-microarch/GPU/platform sensing — deliberately
  conservative on CPU: an unrecognized SKU falls back to the generic x86-64-v2/v3
  feature-level build rather than guessing a specific codename wrong, since a wrong
  guess risks a kernel using instructions the CPU doesn't actually support.
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
  hardware.rs      CPU/GPU/platform detection -> Profile (3-axis combo)
  kernel.rs        Profile -> matching store atom, with popularity-aware fallback
  disk.rs          lsblk-backed disk enumeration
  partition.rs     layout planning + swap sizing + apply() + mount_target()
  stage3.rs        resolve/download/unpack official stage3
  bootloader.rs    Limine config generation + install
  network.rs       iwd client (zbus) + ethernet link check
  store.rs         wires target root to the distro's overlay/binhost
  fstab.rs         UUID-based /etc/fstab generation
  install.rs       orchestrates all of the above into one run, reporting Progress

installer-cli/    ratatui TUI binary — minimal ISO only
installer-gui/    gtk4-rs + libadwaita binary — main ISO only
```

Both frontends call into `installer-core` only; no logic is duplicated
between them.

## Status

Workspace builds and passes clippy clean across all three crates, 15 unit tests
passing. Hardware detection was live-verified on this machine (Victus 16,
i7-14650HX + RTX 4070): `Profile::detect()` produces
`intel-raptorlake-nvidia-laptop`, an exact match against rank #3 of the
real 304-kernel build.

Implemented for real (not stubs): CPU/GPU/platform hardware detection,
disk listing, stage3 resolve/download/verify/unpack, full partitioning
(`parted`/`mkfs.*`/btrfs subvolumes) + mount, Limine deploy, iwd D-Bus network
client (open networks only — see below), store repo/binhost config + chroot
sync, fstab generation, and `install::run` tying all of it into one pipeline
with progress events.

CLI (`installer-cli`) wizard is fully wired end to end: Network → DiskSelect
(live hardware profile + disk list) → Confirm → Installing, which spawns
`install::run` in the background and streams its `Progress` into a live log.
Requires `GENTOO_STORE_BINHOST_URL` / `GENTOO_STORE_OVERLAY_URL` in the
environment — the store isn't published under a fixed URL yet (still a local
overlay, see `~/portage-store-architecture.md`), so these are deliberately not
hardcoded; Confirm shows a clear error instead of pointing at a URL that
doesn't exist. GUI (`installer-gui`) has Welcome → disk-select wired and
smoke-tested on a live niri session; Confirm/Installing pages not built yet.

Not yet done: GUI's Confirm/Installing pages, wifi-connect UI in either
frontend, the iwd passphrase agent (secured-network connect currently
hangs/fails — open networks work), and real binhost/overlay URLs once the
store is published externally. Package-set selection (what gets installed
beyond base system — DE/WM flavors, etc.) is intentionally not designed yet.
