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

Workspace builds and passes clippy clean across all three crates. Implemented
for real (not stubs):

- `hardware`: CPU vendor, GPU vendor (`lspci -nn` parsing, discrete-over-integrated
  preference), RAM, laptop chassis detection
- `disk`: real disk enumeration via `lsblk -J`
- `stage3`: resolves the current autobuild from `latest-stage3-amd64-openrc.txt`,
  pulls the SHA256 out of the companion `.DIGESTS` file, streams the download with
  a running hash check
- `partition`: full `parted`/`mkfs.*`/`btrfs subvolume create` sequence and
  `mount_target` for the post-format mount layout
- `bootloader`: Limine artifact deployment (BIOS vs UEFI path) + config generation
- `network`: real `net.connman.iwd` D-Bus calls (scan, ordered networks, connect) —
  open networks work end to end; secured networks need an iwd Agent object
  registered on the bus to hand back the passphrase, not implemented yet
- `store`: writes `repos.conf`/`binrepos.conf` into the target root and
  chroot-syncs; binhost `Packages` index parsing for atom listing

CLI (`installer-cli`) wizard now walks Network → DiskSelect with live data
(ethernet link check, hardware profile, disk list) and a Confirm screen; GUI
(`installer-gui`) has a working Welcome → disk-select `AdwNavigationView` flow,
smoke-tested on a live niri session (screenshot confirmed the window renders
and the disk-select page populates from `installer-core`).

Not yet done: the Installing step in both frontends (wiring the above into an
actual run with progress reporting), the wifi-connect UI in either frontend,
fstab generation, and the iwd passphrase agent. Package-set selection (what
gets installed beyond base system — DE/WM flavors, etc.) is intentionally not
designed yet.
