# Gentoo Installer

Installer for a Gentoo-based distro built on top of a custom Portage store
(binhost + overlay, see `../portage-store`). Two ISOs, two frontends over one
shared core:

- **minimal ISO** — no GUI, no WM/DE at all. `installer-cli` (ratatui TUI).
- **main ISO** — niri + a shell package preinstalled on the target,
  GTK4 + libadwaita installer (`installer-gui`).

Both frontends and `--headless` run the same phase pipeline (`installer-core/src/phase/`), which writes a journal so
a failed install can be resumed. Everything shells out through a `CommandRunner`, so it is tested without root or a real
disk. OpenRC only.

## Base system

- **Stage3**: official Gentoo `amd64-openrc` autobuild — multilib, non-hardened,
  OpenRC init. Not building a custom stage3 via catalyst; the distro's identity
  lives in the overlay/binhost and in the installer itself, not the base tarball.
  (Deviation from the target spec, which describes unsquashing a single
  pre-built image — this codebase has no squashfs pipeline; see `phase::deploy`'s
  doc comment.)
- **Kernel**: no on-device compilation, and no `emerge` either — deployment is a
  direct file copy (`kernel::deploy`), not a package install. Precompiled kernel
  binaries are built ahead of time by `kernel-configs/matrix-build.sh` (real
  script, `../kernel-releases/`) across a 3-axis matrix — `<cpu>-<gpu>-<platform>`
  — e.g. `intel-raptorlake-nvidia-laptop`. EC/WMI vendor and modem support are
  *not* axes: every build always includes all vendor EC/modem drivers as modules
  (`ec/all` + `modem/all`), since `--skip-modules` means they never affected the
  bzImage anyway. `server` was also dropped from the platform axis (out of scope
  for this distro). 378 possible cpu×gpu×platform combinations exist (18 CPU
  codenames × 7 GPU × 3 platform), 304 are actually built by popularity
  (`gen-popular-targets.py`'s scoring), so `installer-core::kernel::resolve`
  degrades through `hardware::Profile::candidates()` (generalize platform → cpu
  → gpu) rather than requiring an exact match. Detection (`installer-core::hardware`)
  is real CPU-microarch/GPU/platform sensing — deliberately conservative on CPU:
  an unrecognized SKU falls back to the generic x86-64-v2/v3 feature-level build
  rather than guessing a specific codename wrong, since a wrong guess risks a
  kernel using instructions the CPU doesn't actually support.
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
- **Portage config**: `git init` in the target's `/etc/portage` with a first
  commit once make.conf/repos/binrepos are written — "what did the installer
  decide" becomes `git log`, "undo it" becomes `git revert`.

## Architecture

```
installer-core/     lib crate — all real logic, no UI code

  phase/             Phase trait, PhaseId, Ctx, Settings, run_phases — the state machine (15 phases, in order)
    preflight.rs        disk/firmware/battery/network checks
    partition.rs        PartitionPhase + FormatPhase (parted, then mkfs)
    mount.rs             MountPhase
    deploy.rs            stage3 unpack + kernel match+deploy (~70% of wall-clock)
    portage_config.rs   hardware-tuned make.conf (written right after Deploy, before any emerge)
    fstab.rs              FstabPhase
    locale.rs             LocalePhase: keyboard, time zone, hostname, locale-gen
    users.rs              UsersPhase: first account + /etc/doas.conf
    gpu.rs                GpuPhase: on-target Nvidia driver when needed
    desktop.rs            DesktopPhase: compositor + Noctalia + the user's session files (skipped with WM=none)
    packages.rs           PackagesPhase: the software groups chosen
    bootloader.rs        Limine
    post_hooks.rs        records /etc/portage history
    finalize.rs          sync + unmount
  install.rs         InstallOptions -> Settings; run(opts, tx, RunMode::{Fresh,Resume}); the frontends' entry point
  event.rs           Event enum (PhaseStarted/Progress/Log/PhaseFinished/Failed/Complete)
  journal.rs         on-disk resume state (/run/installer/journal.json)
  command.rs         CommandRunner trait + Real/Fake — every shell-out goes through this
  detect.rs          env/hardware inference for make.conf + confirm screen
  gpu.rs             every graphics adapter from lspci: VIDEO_CARDS, discrete-first render device
  autodetect.rs      locales + time zone guessed from the keyboard layout

  Step logic (called by the phases):
  hardware.rs        CPU/GPU/platform detection -> Profile (3-axis combo)
  kernel.rs          Profile -> matching build + direct-copy deploy (no emerge)
  disk.rs            lsblk-backed disk enumeration
  partition.rs       layout planning + swap sizing + create/format/mount
  stage3.rs          resolve/download/verify/unpack a stage3 (official, custom URL, or bundled on the medium)
  bootloader.rs      Limine config generation + install
  store.rs           writes repos.conf/binrepos.conf, git-clones the overlay
  fstab.rs           UUID-based /etc/fstab generation
  locale.rs          locale.gen + locale-gen + LANG, hostname
  account.rs         useradd (-R target) + doas.conf; timezone.rs / keyboard.rs likewise
  make_conf.rs       hardware-tuned /etc/portage/make.conf (-march=, MAKEOPTS, ...)
  gpu_driver.rs       on-target Nvidia driver build against the exact kernel
  wm.rs              compositor + Noctalia install, preset config from wm-configs, GPU tuning of those configs
  services.rs        rc-update for dbus, seatd, iwd, ...
  packages.rs        software groups
  chroot_emerge.rs   shared chroot/emerge bootstrap (resolv.conf, tree sync, bind mounts)
  http.rs            tiny shared download helper
  network.rs         iwd client (zbus) + ethernet link check
  config.rs          StoreEnv (env-var-sourced store config, shared by frontends)

installer-cli/    ratatui TUI (and `--headless`) — minimal ISO
installer-gui/    gtk4-rs + libadwaita — main ISO
```

Both frontends call into `installer-core` only; no step logic lives in them.

## Status

Workspace builds and passes `cargo fmt --check`, build and tests in CI (GitHub Actions): 138 core and 17 CLI tests, plus
tests that need root, a stage3 or QEMU and are run by hand (see below).

What works, each exercised in a VM or QEMU rather than only in unit tests:

- **One pipeline.** TUI, GUI and `--headless` run the same phases. A failed install can be **resumed** (`r` in the TUI, a
  button in the GUI, `--resume`): destructive phases are skipped on the journal alone, the others only if their result is
  still on the disk. The VM test of this found that Deploy could not run twice (fixed: stage-unpacked marker, overlay re-clone).
- **Hardware.** `Profile::detect()` picks a kernel combo from CPU, GPU and form factor; `make.conf` gets a real `-march=`,
  `MAKEOPTS`, `CPU_FLAGS_X86` and `VIDEO_CARDS` for **every** adapter (`amdgpu radeonsi`, `intel`, `nvidia`, `virgl` in a VM).
  The compositor renders on the discrete GPU unless `GENTOO_INSTALLER_RENDER=integrated`: niri's `render-drm-device`,
  `WLR_DRM_DEVICES`/`AQ_DRM_DEVICES` for the others, and Noctalia's shared GL context off for the proprietary NVIDIA driver.
- **Language and zone** follow the keyboard layout (`ua` gives `uk_UA` + `en_US` and `Europe/Kyiv`); only unambiguous countries get a zone.
- **Desktop** (`wm`): Noctalia plus niri (default), Hyprland, Sway, Labwc, MangoWC or dwl, from a wm-configs preset;
  `GENTOO_INSTALLER_WM=none` installs no desktop.
- **Kernels** come from a per-hardware store; a full install used `generic-x86-64-v3-none-desktop` from it.

Needs `GENTOO_STORE_BINHOST_URL` / `GENTOO_STORE_OVERLAY_URL` for real runs (the store has no fixed public URL yet);
`GENTOO_INSTALLER_SIMULATE=1` gives a click-through with no disk, network or chroot.

**Not done:** `Initramfs` is unused (the kernel store ships ready-made initramfs-less builds); a plan-JSON driver;
preset/edition integration from `portage_store`; the iwd passphrase agent is only exercised against a fake (no Wi-Fi radio in the VM);
LUKS; Secure Boot for the *installed* system; a translated installer UI (only locale defaults are guessed); any real hardware.

**Known gap**: `CommandRunner` covers shell-outs, not HTTP — `stage3::download`, `store::list_binhost_atoms` and `kernel::deploy`'s
fetches are real `reqwest` calls with no fake layer, so Deploy is tested in a VM, not in `cargo test`.

## Images, Ventoy, old CPUs

`iso/` builds the live images (see `iso/README.md`). Published as [Simple Linux](https://github.com/tarilka0gg/simple-linux).
Things the tests of the images found, now fixed: the installer binaries carried a "needs x86-64-v3" note from the build
machine's libc and did not start on pre-Haswell CPUs (`iso/strip-isa-note.sh`); the images carry `boot/grub/grub.cfg` so
**Ventoy** can boot them (grub2 mode; BIOS normal mode does not work); `SECUREBOOT_KEYS=` signs Limine and pins the config,
kernel and initramfs by hash (`iso/secureboot/`).

## Packaging

`packaging/package.sh` builds `dist/gentoo-installer-<version>-linux-<arch>.tar.gz` (binaries, desktop entries, icon,
metainfo, `install.sh`); `packaging/gentoo/` has a live ebuild. See `packaging/README.md`.

## Known gaps found while adding `Locale`/`Users`

- ~~**The installed system had no way to become root.**~~ **Fixed and verified on a
  real stage3.** Root is deliberately left locked (`*` in `/etc/shadow`) and the first
  user goes in `wheel`, but a stage3 ships neither `sudo` nor `doas`, so the account
  could never administer the machine. `account::configure_privilege` now writes
  `/etc/doas.conf` (`permit :wheel`) and `account::install_doas` emerges
  `app-admin/doas`; both `UsersPhase` and `install.rs` (what the frontends
  drive) call them. A real-target test has a `wheel` user enter their password on a
  pty and get a root shell, and checks that a wrong password, and a user outside
  `wheel`, do not. The rule has no `persist` on purpose: Gentoo builds `doas` with
  `-persist` by default, where the keyword is accepted and silently ignored, and even
  with `USE=persist` the password was still asked on every call in the chroot test.
- **Frontends: hostname and locale are wired, but not asked.** `InstallOptions` now has
  `hostname` and `locales`, and `install.rs` applies them (`locale::apply_hostname`,
  then `locale::apply`). The TUI reads `GENTOO_INSTALLER_HOSTNAME` / `GENTOO_INSTALLER_LOCALES`
  (comma-separated); the GUI has no page for them yet and uses `gentoo` / `en_US.UTF-8`.
  The phase system (`Settings`) is still not driven by either frontend.
- **`locale-gen` needs `/proc`.** Found by running the step against a real stage3
  (it compiles the locales, then aborts on `findmnt: can't read /proc/mounts`,
  leaving `locale -a` at `C, C.utf8, POSIX`); no `FakeCommandRunner` test could have
  seen it. `locale::apply` now bind-mounts `/proc`, `/sys`, `/dev` around it and always
  unmounts.
- **`useradd -p <hash>` skips the password-quality check** (`pam_passwdqc`) that
  `chpasswd` enforces, so the installer accepts passwords the installed system would
  refuse to set. Unchanged; noted because a weak password on an account that can
  `doas` to root matters more than it used to.

## Testing against a real stage3

`installer-core/tests/real_target.rs` runs the `Locale`/`Users` steps with the real
command runner against an unpacked stage3 and asks the *target's own tools* what came
out (`locale -a`, `id -nG`, `getent shadow`, a password-vs-hash check, and a `doas`
login on a pty). Five tests, `#[ignore]`d — they need root, network for the `doas`
ones, and stage3 trees on btrfs/xfs (each test works on a `cp --reflink` copy):

```bash
# 1. a pristine stage3, and a second copy with the Portage tree synced (no doas in either)
tar xpf stage3-amd64-openrc-*.tar.xz --xattrs-include='*.*' --numeric-owner -C /path/pristine
cp -a --reflink=always /path/pristine /path/pristine-synced      # then run emerge-webrsync
                                                                   # in it, in an unshare'd chroot
# 2. build as your user, run as root
cargo test -p installer-core --test real_target --no-run
sudo GENTOO_INSTALLER_STAGE3=/path/pristine GENTOO_INSTALLER_STAGE3_SYNCED=/path/pristine-synced \
     unshare --mount --propagation private \
     target/debug/deps/real_target-<hash> --ignored --test-threads=1
```

About 40 s in total. `unshare` keeps the bind mounts these steps make out of the host's mount table even if
a test dies half-way, and each copy is deleted by a guard that **refuses to delete while
anything is still mounted under it** (`remove_dir_all` does not stop at mount points and
would otherwise recurse into the host's real `/dev`).

## OpenRC only

This distribution has no systemd, and the code must not assume one. An audit of every external call and file path
found three places that did (all fixed): the GUI's *Restart* ran `systemctl reboot` (now `reboot`); the keyboard
layout was read from systemd's `/etc/vconsole.conf` (now OpenRC's `/etc/conf.d/keymaps`, the systemd file only as a
fallback); and — the real gap — nothing ever enabled a service on the installed system. `installer-core/src/services.rs`
is now the one place that does, with `chroot <target> rc-update add <svc> default` (names validated, safe to repeat,
tested against a real stage3). It is used for: `dbus` + `seatd` and the user's `seat,video,input,audio,render` groups after a
compositor install (without a seat manager niri cannot open the GPU), and `dbus` + `iwd` for the new default package group
*Wi-Fi and firmware* (`iwd`, `linux-firmware`, with the firmware licence accepted in `package.license`), so an installed
laptop can reach the network without Ethernet. Everything else that touches init is plain files that OpenRC reads:
`/etc/conf.d/hostname`, `/etc/conf.d/keymaps`, `/etc/env.d/02locale`, `/etc/timezone`.

After a finished install the pipeline runs `sync` and `umount -R` on the target, and the frontends offer a reboot: the TUI
stays on its Done screen (Enter reboots, but only if no step failed; `q` leaves) and the GUI button runs `reboot`.
Not run end to end yet: the desktop and Wi-Fi steps on a real target (they need a long compile and the network).

## A full run through the TUI, in a VM

The minimal image was booted in QEMU, the TUI driven with the keyboard exactly as a person would (network → disk → *new*
account screen → confirm), against a local test store, with the desktop (niri + Noctalia, 157 packages, most of them binary
from Gentoo's binhost) and the Wi-Fi group. It finished with "Install complete", offered a reboot, and the reboot happened.
The installed disk then booted by itself; the created user logged in with fish, in `wheel`, `seat`, `video`, `render`,
`input` and `audio`; `dbus`, `seatd` and `iwd` were in the default runlevel and running; `niri`, `noctalia`, `iwctl` and 686
firmware entries were present; `doas` asked for the user's password.

The run also found, and the code now fixes (each with a test): the TUI read the wm-configs URL from a variable the store config
does not use and then hid the resulting error (it only reported errors after a `Done` that a failure never sends); the user's
login shell is fish on a custom stage and fish ignores `~/.bash_profile`, so niri was never started (a fish snippet is written
now); and nothing created `XDG_RUNTIME_DIR` (`/run/user/<uid>`), because there is neither systemd nor elogind — an
`/etc/local.d` script does it at boot, with a fallback under `~/.cache`.

With those files in place the session did start (`niri --session` running, seatd up, the runtime directory owned by the user),
but on that VM niri found no output: the target's Mesa has `iris`/`radeonsi`/`nouveau`/`swrast` and no driver for the virtual GPU,
and `make.conf` has no `VIDEO_CARDS` when the hardware is a VM. So **a picture of the installed desktop has not been seen**, on real
hardware or in the VM. Real GPUs have drivers in that Mesa build, but that is an expectation, not a test.

## Wi-Fi in the TUI

With no Ethernet link the TUI scans through `iwd` (the same `IwdClient` the GUI uses), lists the networks
strongest first, asks for the passphrase (masked, 8–63 characters, `q` and Esc are plain input while typing),
and offers rescan and skip. The screen is a pure state machine (`installer-cli/src/wifi.rs`, 9 tests); the real
`iwd` round trip has not been exercised in a VM (there is no Wi-Fi radio there).

## Headless install and what a real run found

`installer-cli --headless` runs the whole phase chain with no screens, configured from the environment
(`GENTOO_INSTALLER_DISK`, `_CONFIRM_ERASE` — must repeat the disk path —, `_USERNAME`, `_PASSWORD`, the
`GENTOO_STORE_*` variables, optionally `_HOSTNAME`/`_LOCALES`/`_TIMEZONE`/`_KEYBOARD`/`_STAGE3_URL`).
`phase::run_all` is the driver: it skips phases whose `is_satisfied` holds, stops at the first failure
and reports it. `--resume` continues the unfinished install this live session remembers (journal in `/run/installer`), skipping what is
already done. `GENTOO_INSTALLER_WM=none` installs no desktop (a console-only system).

Running it for real in a VM (live minimal ISO, blank virtio disk, a local test store with the live
kernel, bundled stage) found four bugs no unit test could: `git commit` in `/etc/portage` needs an
identity; `Deploy` counted a bare unpacked stage3 as done, so a retry skipped the kernel; `Fstab`
counted the stage3's own comment-only `/etc/fstab` as written, so the system booted with no `/home`,
`/var`, `/boot` or swap; and the kernel `.config` had dropped `FRAMEBUFFER_CONSOLE`. All fixed, each with a test. The resulting system booted by itself
from the installed disk; see `iso/README.md` for what was and was not covered.

## Custom stage3

The installer normally unpacks Gentoo's latest stage3. `GENTOO_INSTALLER_STAGE3_URL` (TUI and GUI;
`InstallOptions::stage3` / `Settings::stage3` in code) points it at your own tarball instead: an
`https://` URL, a `file://` path or a plain absolute path, with `GENTOO_INSTALLER_STAGE3_SHA512`
checked while it downloads (a mismatch aborts before anything is unpacked). Without the variable the
installer looks for a stage3 shipped on the live medium (`/run/initramfs/live/stage`, see
`iso/README.md`) and only then falls back to Gentoo's latest from the mirror.

`iso/make-stage.sh` builds one: Gentoo's stage3 + fish, eza, dust, gping and micro (installed from
binary packages, nothing compiled there), `nano` removed, the house aliases in
`/etc/fish/conf.d/10-house.fish`, and the Portage tree and caches stripped again. The result is
240 MB (Gentoo's own is 265 MB). The first user is created with fish as login shell when the stage
has it, otherwise bash; root stays locked either way.

Tested end to end with the real `tar`/`useradd` (`a_custom_stage_is_fetched_verified_unpacked_and_gives_the_user_fish`
in `installer-core/tests/real_target.rs`: digest mismatch refused, tarball unpacked, user's shell is
fish). A full install from the custom stage has not been run (the store has no kernels).

## Choices: desktop, graphics driver, software

Advanced setup (GUI) or environment variables (TUI) choose three things:

- **Graphics driver** — `InstallOptions::gpu_override` (`GENTOO_INSTALLER_GPU=nvidia|nouveau|amd|intel|xe|none`).
  Detection stays the default; the override picks the kernel build in the store and whether
  the proprietary NVIDIA module is compiled.
- **Software** — `installer-core/src/packages.rs` holds the groups (terminal, browser, CLI tools,
  development, media, graphics, messaging, office, gaming tools). `GENTOO_INSTALLER_PACKAGES=terminal,browser,dev`
  (unset = the defaults: terminal, browser, CLI tools). The emerge runs with
  `--autounmask-write --autounmask-continue`: on a bare stage3 nearly every desktop package
  needs a point USE change, and every atom was checked with `emerge -f` on a real stage3.
  Steam and Discord are deliberately not offered (overlay/multilib/licence decisions).
- **Desktop** — niri, Hyprland, Sway, Labwc, MangoWC and dwl (`GENTOO_INSTALLER_WM=…`). dwl's config is a C
  header: the preset's `config.h` goes to `/etc/portage/savedconfig/gui-wm/dwl` with `USE=savedconfig` and
  is compiled in. Scroll and Triad from `gentoo-wm-configs` have no ebuild in `gentoo`, `guru` or the local
  overlays, so they cannot be installed yet.

## License

[GPL-2.0-or-later](LICENSE), matching Portage, Gentoo and this project's sibling
[portage-store](https://github.com/tarilka0gg/portage-store).
