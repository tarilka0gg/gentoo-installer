# Gentoo Installer

Installer for a Gentoo-based distro built on top of a custom Portage store
(binhost + overlay, see `../portage-store`). Two ISOs, two frontends over one
shared core:

- **minimal ISO** — no GUI, no WM/DE at all. `installer-cli` (ratatui TUI).
- **main ISO** — niri + a shell package preinstalled on the target,
  GTK4 + libadwaita installer (`installer-gui`).

Target architecture is a formal build spec (`INSTALLER-SPEC.md`-equivalent,
kept in conversation history) built around a linear `Phase` state machine with
journal/resume, an `Event` stream, and a `CommandRunner` abstraction for
testing without root or real disks. `installer-core` is partway migrated onto
it — see **Architecture** below for what's real vs. still legacy.

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

  Spec-aligned (new):
  phase/             Phase trait + PhaseId + Ctx — the state machine
    preflight.rs        disk/firmware/battery/network checks
    partition.rs        PartitionPhase + FormatPhase (parted, then mkfs)
    mount.rs             MountPhase
    deploy.rs            stage3 unpack + kernel match+deploy (~70% of wall-clock)
    fstab.rs              FstabPhase
    locale.rs             LocalePhase: keyboard, time zone, hostname, locale-gen
    users.rs              UsersPhase: first account + /etc/doas.conf
    portage_config.rs   make.conf + git init/commit
    bootloader.rs        Limine
    finalize.rs          sync + unmount
  event.rs           Event enum (PhaseStarted/Progress/Log/PhaseFinished/Failed/Complete)
  journal.rs         on-disk resume state (JSON), is_satisfied()-driven skip logic
  command.rs         CommandRunner trait + Real/Fake — every shell-out goes through this
  detect.rs          env/hardware inference for make.conf + confirm screen

  Step logic (called by phases above, and still by legacy install.rs):
  hardware.rs        CPU/GPU/platform detection -> Profile (3-axis combo)
  kernel.rs          Profile -> matching build + direct-copy deploy (no emerge)
  disk.rs            lsblk-backed disk enumeration
  partition.rs       layout planning + swap sizing + create/format/mount
  stage3.rs          resolve/download/verify/unpack official stage3
  bootloader.rs      Limine config generation + install
  store.rs           writes repos.conf/binrepos.conf, git-clones the overlay
  fstab.rs           UUID-based /etc/fstab generation
  locale.rs          locale.gen + locale-gen + LANG, hostname
  account.rs         useradd (-R target) + doas.conf; timezone.rs / keyboard.rs likewise
  make_conf.rs       hardware-tuned /etc/portage/make.conf (-march=, MAKEOPTS, ...)
  gpu_driver.rs       on-target Nvidia driver build against the exact kernel
  wm.rs              compositor + Noctalia install, preset config from wm-configs
  chroot_emerge.rs   shared chroot/emerge bootstrap (resolv.conf, tree sync, bind mounts)
  http.rs            tiny shared download helper
  network.rs         iwd client (zbus) + ethernet link check
  config.rs          StoreEnv (env-var-sourced store config, shared by frontends)

  Legacy (still what installer-cli/installer-gui actually drive):
  install.rs         monolithic orchestrator, same step functions as phase/, no journal

installer-cli/    ratatui TUI binary — minimal ISO only, drives install.rs today
installer-gui/    gtk4-rs + libadwaita binary — main ISO only, drives install.rs today
```

Both frontends call into `installer-core` only; no step logic is duplicated
between the phase system and `install.rs` — phases call the exact same
functions (`partition::create_partitions`, `stage3::download`, `kernel::deploy`,
...) that `install.rs` does, just with journal/resume/event-stream bookkeeping
around them.

**Why two orchestrators right now**: the target spec's own build order says
build the phase/journal/CommandRunner foundation and prove it headless *before*
touching the GUI. `installer-gui` predates that spec and is built on
libadwaita's `AdwNavigationView`/`AdwStatusPage` — which the spec explicitly
rules out for the primary path (GNOME layout grammar, not this installer's
interaction model). Rewriting it is deliberately the last step, not skipped.

## Status

Workspace builds and passes clippy clean across all three crates, 82 unit/
integration tests passing (plus 5 real-stage3 tests, run separately), including a `Partition→Format→Mount→Fstab`
end-to-end run against `FakeCommandRunner` with no root and no real disk.
Hardware detection was live-verified on this machine (Victus 16, i7-14650HX +
RTX 4070): `Profile::detect()` produces `intel-raptorlake-nvidia-laptop`, an
exact match against rank #3 of the real 304-kernel build.

`make.conf` generation (`make_conf`) and desktop install (`wm`) are both real
and wired into `install::run`: `make.conf` gets a hardware-real `-march=`,
`MAKEOPTS` sized to core count, and `CPU_FLAGS_X86`/`VIDEO_CARDS` from
`detect::gather`, with `Advanced` setup choosing `-O2`/`-O3` and
binary-vs-source packages (`GENTOO_INSTALLER_OPT_LEVEL`/`_PACKAGE_MODE` env
vars in the CLI, dedicated pages in the GUI). `wm` installs Noctalia plus one
of niri (default)/Hyprland/Sway/Labwc/MangoWC — every atom/overlay/keyword
requirement live-verified against a real synced tree, GURU, and hyproverlay —
then clones and applies `wm_configs_git_url`'s preset for the chosen
compositor. Both share new `chroot_emerge` bootstrap plumbing (resolv.conf,
Portage tree sync, bind mounts) with `gpu_driver`, which itself gained real
`package.license`/`package.accept_keywords`/`package.use` overrides — emerging
`nvidia-drivers` unconditionally failed before this, since Portage never
auto-accepts its license or resolves its USE deps without them.

CLI (`installer-cli`) and GUI (`installer-gui`) both drive the legacy
`install.rs` orchestrator end to end: Network → DiskSelect → Confirm →
Installing, with `GENTOO_INSTALLER_SIMULATE=1` for a real click-through with no
disk/network/chroot access (fakes the `Progress` sequence with sleeps; hardware
detection still runs for real). Requires `GENTOO_STORE_BINHOST_URL` /
`GENTOO_STORE_OVERLAY_URL` in the environment for real runs — the store isn't
published under a fixed URL yet (still a local overlay, see
`~/portage-store-architecture.md`), so these are deliberately not hardcoded.

**Known gap**: `CommandRunner` covers shell-outs, not HTTP — `stage3::download`,
`store::list_binhost_atoms`, and `kernel::deploy`'s fetches are real `reqwest`
calls with no fake/injectable layer yet, so `Deploy`/`PortageConfig` phases
can't be exercised in the no-network test environment spec §10 requires. The
`phase::tests` integration test covers `Partition`→`Fstab` for exactly this
reason — that's as far as the current fake goes.

Not yet done (see spec for the full list): `installer-cli`/`installer-gui`
driving the new phase/journal system instead of legacy `install.rs`; a
non-interactive `installer-cli` plan-JSON driver (spec's step 4, the one that
proves the installer works headless); `Initramfs`/`PostHooks` phases (dracut, machine-id/eix seeding — genuinely new
territory, nothing here does this yet); the
non-libadwaita `installer-gtk` rewrite and its 13-page flow; preset/edition
integration from `portage_store`; a WM picker in `installer-cli`'s TUI (it
reads `GENTOO_INSTALLER_WM`/`_WM_CONFIGS_URL` env vars instead — same gap
already true for account creation); the iwd passphrase agent (secured-network
connect currently hangs/fails — open networks work); wifi-connect UI in either
frontend; and real binhost/overlay URLs once the store is published
externally.

### Known gaps found while adding `Locale`/`Users`

- ~~**The installed system had no way to become root.**~~ **Fixed and verified on a
  real stage3.** Root is deliberately left locked (`*` in `/etc/shadow`) and the first
  user goes in `wheel`, but a stage3 ships neither `sudo` nor `doas`, so the account
  could never administer the machine. `account::configure_privilege` now writes
  `/etc/doas.conf` (`permit :wheel`) and `account::install_doas` emerges
  `app-admin/doas`; both `UsersPhase` and the legacy `install.rs` (what the frontends
  drive) call them. A real-target test has a `wheel` user enter their password on a
  pty and get a root shell, and checks that a wrong password, and a user outside
  `wheel`, do not. The rule has no `persist` on purpose: Gentoo builds `doas` with
  `-persist` by default, where the keyword is accepted and silently ignored, and even
  with `USE=persist` the password was still asked on every call in the chroot test.
- **Frontends: hostname and locale are wired, but not asked.** `InstallOptions` now has
  `hostname` and `locales`, and the legacy `install.rs` applies them (`locale::apply_hostname`,
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

### Testing against a real stage3

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

## Custom stage3

The installer normally unpacks Gentoo's latest stage3. `GENTOO_INSTALLER_STAGE3_URL` (TUI and GUI;
`InstallOptions::stage3` / `Settings::stage3` in code) points it at your own tarball instead: an
`https://` URL, a `file://` path or a plain absolute path, with `GENTOO_INSTALLER_STAGE3_SHA512`
checked while it downloads (a mismatch aborts before anything is unpacked).

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
