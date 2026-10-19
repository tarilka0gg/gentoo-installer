# Gentoo Installer

The installer and the image build scripts behind [Simple Linux](https://github.com/tarilka0gg/simple-linux),
a Gentoo-based live system. It installs Gentoo (OpenRC, Limine) onto a disk in about ten minutes, from binary
packages, with a kernel picked for the machine's hardware.

- **`installer-cli`**: ratatui text installer (and `--headless`), used by the minimal image.
- **`installer-gui`**: GTK4 + libadwaita wizard, used by the GUI image (niri + Noctalia).
- **`installer-core`**: everything that actually happens. Both frontends only call into it.
- **`profiles/`, `slp-profiles/`, `slp/`**: the flavors and software groups as data, the library that reads them and the `slp` command.
- **`iso/`**: builds the live images, the offline payload and the release.

License: [GPL-2.0-or-later](LICENSE).

## What it does

- **One pipeline.** The TUI, the GUI and `--headless` run the same 16 phases (`installer-core/src/phase/`):
  preflight, partition, format, mount, deploy, portage config, fstab, locale, users, kernel build (optional), GPU, desktop,
  packages, bootloader, post hooks, finalize. A journal (`/run/installer/journal.json`) lets a failed install be
  **resumed**: destructive phases are skipped on the journal alone, the others only if their result is still on the disk. The journal also keeps the
  disk layout the run made, so a resume never plans again from a disk that already has the new partitions.
  Every shell-out goes through a `CommandRunner`, so the logic is unit-tested without root or a disk.
- **Hardware-matched kernel.** `hardware::Profile::detect()` reads CPU microarchitecture, GPU vendor(s) and
  laptop/desktop, and `kernel::resolve` picks a prebuilt kernel from the store, falling back step by step (platform,
  then CPU, then GPU) down to a generic x86-64 v2/v3 build. Nothing is compiled on the machine, and `emerge` is not
  used for the kernel: deployment is a file copy plus its modules. An unrecognised CPU gets the generic build rather than
  a guess that could use instructions it lacks.
- **Install next to other systems.** Besides erasing a disk, the disk page can use the free space of a GPT disk and keep what is there
  (`partition::Placement::FreeSpace`): you choose how many GiB to give, the new swap/root/home partitions are made with `sgdisk` at the
  lowest unused partition numbers inside the free region, an EFI system partition that is already there is mounted and **never formatted**,
  and the other systems found on it (`EFI/<vendor>/…efi`: Windows, Ubuntu, …) get chainload entries in `limine.conf`; the installer also
  registers its own UEFI boot entry (`efibootmgr`) and keeps a previous `BOOTX64.EFI`. The page shows what is on the disk now and
  offers GParted for making room (shrinking is left to it). Headless: `GENTOO_INSTALLER_PLACEMENT=free` (+ `_FREE_GIB`).
- **Keyboard and time zone.** Several layouts (up to four) with the key that switches them (Caps Lock by default; real Caps Lock is
  Shift+Caps Lock); on the live medium the chosen layouts are applied to the running niri at once, and they are written into the installed
  niri's `xkb {}` and `/etc/X11/xorg.conf.d/00-keyboard.conf` (the console takes the first). The time zone page searches cities, capitals
  and countries (`zone.tab`), and the zone is found from the connection (one request to ipapi.co, in the background, never on an offline image
  and never over a zone the person picked).
- **Graphics.** The page names every card found (vendor and model, integrated or discrete), lets you pick the card the desktop is drawn on
  when there are several, and folds the driver choice into one expander row.
- **Optional kernel for this computer.** Advanced → Optimization → "Build the kernel for this computer" (off by default; env
  `GENTOO_INSTALLER_KERNEL_BUILD=1`). When the store has no kernel for *exactly* this hardware, `kernel_build.rs` fetches Linux 7.1.8 (checksum from
  kernel.org) and the [kernel-matrix](https://github.com/tarilka0gg/kernel-matrix) (`GENTOO_KERNEL_MATRIX_URL` replaces it), merges the seed and
  the fragments in `build-config.sh`'s order, and builds on the new system (gcc, then clang if gcc fails); the result replaces the store's kernel
  and its module tree. A failure keeps the ready-made kernel. `/etc/simple-linux/local-kernel` marks the result and `simple-linux-update` leaves it alone.
  Checked in a VM (7 jobs): built and booted, and the build took far less than the planning figure of 10 to 60 minutes (it depends on the machine); about 10 GB of disk.
- **Binary packages.** Software comes prebuilt from Gentoo's binhost and from the project's own
  ([simple-linux-binhost](https://github.com/tarilka0gg/simple-linux-binhost): niri, Noctalia, ghostty, packages whose USE
  the installer changes). A binary is used only if its USE flags match, so what the installer asks for is pinned in
  `installer-core/assets/binhost/` and the binhost is built from the same files. A full install of every software group
  merged 263 packages in a VM, compiled none, and took about 15 minutes (the default groups: 261 packages, about 10).
- **Offline install.** The `*-offline.iso` images carry the stage3, kernels, packages, Portage tree, configs and source
  files on the medium; `offline.rs` serves them on `127.0.0.1` and points every URL at it (see below).
- **Language and zone** are guessed from the keyboard layout (`ua` gives `uk_UA` + `en_US` and `Europe/Kyiv`).
- **Every decision is a `git log`.** `/etc/portage` of the target is a git repo with a commit once `make.conf`, repos and
  binrepos are written.
- **Desktop**: Noctalia + niri by default; Hyprland, Sway, Labwc, MangoWC and dwl from `simple-linux-configs` presets.
  Scroll and Triad have no ebuild yet. `GENTOO_INSTALLER_WM=none` installs no desktop.
- **Extras copied from the live image**: [ustan](https://github.com/tarilka0gg/ustan), Portage Store and
  [tui-shim](https://github.com/tarilka0gg/tui-shim) (terminal programs as desktop apps).

## Screenshots

The graphical installer, page by page (0.2.33, in a virtual machine; the images live in the [simple-linux](https://github.com/tarilka0gg/simple-linux#the-installer) repository).

<table>
  <tr>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/01-welcome.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/01-welcome.png" alt="Welcome" width="100%"></a></td>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/02-keyboard.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/02-keyboard.png" alt="Keyboard layouts" width="100%"></a></td>
  </tr>
  <tr>
    <td><b>Welcome</b><br><sub>The installer's first screen: disk, kernel and boot are decided for you.</sub></td>
    <td><b>Keyboard layouts</b><br><sub>Add every layout you type; the first is the main one, and a box lets you test them.</sub></td>
  </tr>
  <tr>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/03-timezone.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/03-timezone.png" alt="Time zone" width="100%"></a></td>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/04-desktop.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/04-desktop.png" alt="Desktop" width="100%"></a></td>
  </tr>
  <tr>
    <td><b>Time zone</b><br><sub>Search for a city, capital or country, or let the internet connection tell.</sub></td>
    <td><b>Desktop</b><br><sub>Pick a compositor (niri, Hyprland, Sway, Labwc, MangoWC, dwl); Noctalia comes with any of them.</sub></td>
  </tr>
  <tr>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/05-graphics.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/05-graphics.png" alt="Graphics driver" width="100%"></a></td>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/06-software.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/06-software.png" alt="Software" width="100%"></a></td>
  </tr>
  <tr>
    <td><b>Graphics driver</b><br><sub>The cards found; the choice picks the kernel build and, for NVIDIA, whether the proprietary driver is compiled.</sub></td>
    <td><b>Software</b><br><sub>A system type (console, desktop, workstation) and groups of programs to tick.</sub></td>
  </tr>
  <tr>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/07-software-expanded.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/07-software-expanded.png" alt="A group opened" width="100%"></a></td>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/08-build-options.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/08-build-options.png" alt="Build options" width="100%"></a></td>
  </tr>
  <tr>
    <td><b>A group opened</b><br><sub>Every program says in a sentence what it is, and many have a screenshot.</sub></td>
    <td><b>Build options</b><br><sub>-O3, building everything from source, building the kernel for this computer: all off by default.</sub></td>
  </tr>
  <tr>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/09-disk.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/09-disk.png" alt="Disk" width="100%"></a></td>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/10-installing.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/10-installing.png" alt="Installing" width="100%"></a></td>
  </tr>
  <tr>
    <td><b>Disk</b><br><sub>Erase the whole disk, or install next to what is already there; disks that are too small are greyed out.</sub></td>
    <td><b>Installing</b><br><sub>Sixteen phases with a progress bar and a live log (this is the simulated run).</sub></td>
  </tr>
  <tr>
    <td width="50%"><a href="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/11-done.png"><img src="https://raw.githubusercontent.com/tarilka0gg/simple-linux/main/docs/img/0.2.33/11-done.png" alt="Done" width="100%"></a></td>
  </tr>
  <tr>
    <td><b>Done</b><br><sub>Restart and the system is ready.</sub></td>
  </tr>
</table>

`GENTOO_INSTALLER_GUI_PAGE=keyboard|timezone|desktop|gpu|disk|software|optimization|network installer-gui` opens the wizard on a page, and `installer-gui --debug-progress` shows the installing and done screens with a simulated run, which is how they were taken.

## Design choices

- **OpenRC only, no systemd.** Services are enabled through `services.rs` (`rc-update add … default`); the keyboard layout is read from
  `/etc/conf.d/keymaps`; `XDG_RUNTIME_DIR` is created by an `/etc/local.d` script because there is no elogind.
- **Stage3**: Gentoo's official `amd64-openrc`, or the project's slimmer one (`iso/make-stage.sh`: fish, eza, dust, gping,
  micro, no nano). `GENTOO_INSTALLER_STAGE3_URL` (+ `_SHA512`, otherwise `<url>.sha512` is fetched) points the installer at any tarball:
  `https://`, `file://` or a path. A digest mismatch aborts before anything is unpacked.
- **Bootloader**: Limine only (BIOS and UEFI). Optional Secure Boot (below).
- **Partitioning** is automatic: 512 MiB ESP, swap 1:1 with RAM clamped to 8–96 GiB, then `/` and `/home` as **separate
  partitions** (system 20 % / home 80 %, 40 / 60 under 128 GiB, never under 20 GiB for the system, no split under 40 GiB). btrfs
  with subvolumes (`@`, `@var`, `@log`, `@home`) by default, ext4 optional. No LUKS yet.
- **Network**: iwd over D-Bus, not NetworkManager. Ethernet is detected by link state so the Wi-Fi step can be skipped.
- **Accounts**: root stays locked, the first user is in `wheel` and gets `doas` (`permit :wheel`, no `persist`). Their login shell is fish when the stage has it.
- **GPU**: every adapter from `lspci` goes into `VIDEO_CARDS`; the compositor renders on the discrete GPU unless
  `GENTOO_INSTALLER_RENDER=integrated`. The proprietary NVIDIA driver is built on the target against the exact kernel.

## Layout

```
installer-core/            library; no UI code
  phase/                   Phase trait, PhaseId, Ctx, Settings, run_all (state machine) + one file per phase
  wm/                      compositor install, presets, session files
  install.rs               InstallOptions -> Settings; run(opts, tx, Fresh|Resume): the frontends' entry point
  event.rs, journal.rs     event stream, resume state
  command.rs               CommandRunner (Real / Fake / Streaming)
  hardware.rs, kernel.rs   detection and kernel match + deploy
  partition.rs, disk.rs, fstab.rs, bootloader.rs
  stage3.rs, store.rs, binhost.rs, offline.rs, packages.rs, chroot_emerge.rs
  make_conf.rs, gpu.rs, gpu_driver.rs, locale.rs, keyboard.rs, timezone.rs, autodetect.rs, account.rs, services.rs
  network.rs               iwd client (zbus) + link check
  ustan.rs, portage_store.rs, tui_shim.rs, sltools.rs   things copied from the live image
  assets/                  binhost USE files, simple-linux-update, …
installer-cli/             TUI + --headless + Wi-Fi screen (a state machine, unit-tested)
installer-gui/             GTK4 wizard: welcome, storage, locale, account, desktop, Wi-Fi, confirm, progress
iso/                       image, offline payload and release scripts (see iso/README.md)
packaging/                 tarball + Gentoo ebuild for the installer itself
```

## Building and testing

```
cargo build --release                 # installer-gui needs GTK4 + libadwaita
cargo fmt --check && cargo test       # about 195 core tests and 17 CLI tests; CI runs the same
```

A few tests need root, a stage3 or QEMU and are `#[ignore]`d:

- `installer-core/tests/real_target.rs` runs the locale/user/doas/custom-stage steps with the real command runner against an unpacked stage3 and asks the target's own tools what
  came out. Run it as root inside `unshare --mount --propagation private`, on btrfs/xfs (each test works on a `cp --reflink` copy, and its cleanup refuses to delete while anything is still mounted under it):
  ```
  tar xpf stage3-amd64-openrc-*.tar.xz --xattrs-include='*.*' --numeric-owner -C /path/pristine
  cargo test -p installer-core --test real_target --no-run
  sudo GENTOO_INSTALLER_STAGE3=/path/pristine GENTOO_INSTALLER_STAGE3_SYNCED=/path/pristine-synced \
       unshare --mount --propagation private target/debug/deps/real_target-<hash> --ignored --test-threads=1
  ```
- `installer-core/tests/boot_qemu.rs` boots an image.

`GENTOO_INSTALLER_SIMULATE=1` gives a click-through with no disk, network or chroot (`GENTOO_INSTALLER_SIMULATE_SLOW=10` slows it down); `installer-gui --debug-progress` previews the progress page.

Most bugs of the project were found by real runs, not unit tests: a headless install in a VM (output sent to the host) from a minimal
image onto a blank disk, then booting the installed disk. `iso/test/` has the helpers (QEMU with a virtual 3D GPU, keyboard driver, local store servers).

## Headless install

`installer-cli --headless` runs the whole chain from environment variables and prints the same event stream the GUI shows:

| Variable | Meaning |
|---|---|
| `GENTOO_INSTALLER_DISK`, `_CONFIRM_ERASE` | target disk; the confirmation must repeat the path |
| `_USERNAME`, `_PASSWORD`, `_HOSTNAME` | first account and host name (default `simple-linux`) |
| `_LOCALES`, `_TIMEZONE`, `_KEYBOARD`, `_KEYBOARD_OPTIONS` | comma-separated locales; layouts as `us,ua` and XKB options (`grp:caps_toggle`); otherwise guessed from the keyboard |
| `_PLACEMENT=free`, `_FREE_GIB` | install into the largest free region of a GPT disk (optionally only that many GiB) and keep what is there |
| `_KERNEL_BUILD=1` | build the kernel for this computer from the kernel matrix when the store has none for it |
| `_PACKAGES` | software group ids (`terminal,browser,dev`…); default: the ticked-by-default groups |
| `_WM` | `niri` (default), `hyprland`, `sway`, `labwc`, `mangowc`, `dwl`, `none` |
| `_GPU` | `nvidia`, `nouveau`, `amd`, `intel`, `xe`, `none`; detection is the default |
| `_RENDER`, `_OPT_LEVEL`, `_SECUREBOOT`, `_PACKAGE_MODE` | render GPU, optimisation level, Secure Boot, binary or source packages |
| `_USTAN=0`, `_STORE=0` | do not copy ustan / Portage Store from the live image |
| `_STAGE3_URL`, `_STAGE3_SHA512` | custom stage3 |
| `GENTOO_STORE_BINHOST_URL`, `_OVERLAY_URL`, `GENTOO_BINHOST_URL`, `GENTOO_WM_CONFIGS_URL`, `GENTOO_KERNEL_MATRIX_URL` | store, project binhost, wm-configs repo, kernel-matrix repo; defaults point at the public ones |

(All except the `GENTOO_STORE_*`, `GENTOO_BINHOST_URL` and `GENTOO_WM_CONFIGS_URL` ones start with `GENTOO_INSTALLER`.)
`--resume` continues the unfinished install this live session remembers.

## Flavors, software groups and `slp`

What gets installed is data, in `profiles/` ([format](profiles/README.md)), compiled into the installer by the `slp-profiles` crate. A system is
one **flavor** (`minimal`, `desktop`, `workstation`; `live` is the ISO's own) plus **mix-ins**, the software groups of the Software page: Wi-Fi,
terminal, browser, CLI tools, files, audio, desktop utilities, disk and hardware tools, Gentoo tools, development, media, graphics, messaging,
office, gaming tools, **Steam**, **Minecraft** (PineconeMC), **Creative Craft apps** and **Craft office apps**; the graphics cards are `gfx-*`
mix-ins. Each group says which packages, USE flags, licences and services it needs, has a sentence for every package and, for some, a
screenshot, all shown on the Software page. The emerge runs with `--autounmask-write --autounmask-continue` (plus `--changed-use` when a
group needs 32-bit libraries, i.e. Steam), and every atom was checked on a real stage3.

Programs Portage does not have come from the profiles' own overlay `slp-local` (`profiles/overlay/`: PineconeMC and the Craft apps, as `-bin`
ebuilds of their GitHub releases, pinned by checksum); Steam comes from the `steam-overlay`, cloned during the install. Groups from these
overlays are not on the offline images and not in the binhost. Discord is not offered (proprietary licence).

The same data drives the **`slp`** command on an installed system (`slp show | list | info | flavor | mix-in | apply | install | rebuild`, like
Funtoo's `epro`); the choice is kept in `/etc/slp/state`.

## Binary install details

`PackageMode::Binary` (the default) writes `FEATURES="getbinpkg"`, `EMERGE_DEFAULT_OPTS` and `DONT_MOUNT_BOOT="1"` (the `mount-boot` check of `linux-firmware` aborts in a chroot).
`binhost::configure` adds the project's host with a higher priority and signature checking on; the key is trusted like `getuto` does it: import,
`import-ownertrust` **and** `--check-trustdb` (without the last step Portage rejects a validly signed package). In binary mode `make.conf` does not set the machine's `CPU_FLAGS_X86`, and
`VIDEO_CARDS` is the binhost's set plus what the hardware needs: either would change the USE of `mesa`, `pixman`, … and Portage would compile them.

The Portage tree is the snapshot the packages were built from (`tree.tar.zst`, published beside them), unpacked into `/var/db/repos`; `emerge-webrsync` is the fallback,
otherwise a newer tree would have versions with no binary. `iso/binhost/` has the builder (`build.sh` in a clean stage3 chroot, `build-variant.sh`, `make-repo.py`, `export-assets.sh`).

## Offline images

`simple-linux-gui-offline.iso` and `simple-linux-minimal-offline.iso` carry a payload (`/run/initramfs/live/offline`, built by `iso/make-offline.sh`): the stage3, the kernel
store, the binary packages of the default groups (indexes cut per host), the Portage tree, source files, `linux-firmware` and a bare `configs.git`.
`offline::activate()` starts a small HTTP server on `127.0.0.1` (GET/HEAD/Range) and sets `GENTOO_STORE_BINHOST_URL`, `GENTOO_BINHOST_URL`, `GENTOO_WM_CONFIGS_URL`; `binhost::configure` points Portage's
`[gentoo]` binrepo and `GENTOO_MIRRORS` at it, and `binhost::restore_online` puts the public URLs back at the end, so the installed system updates normally.
To fit GitHub's 2 GiB asset limit the payload has no Zen, no optional groups (`NOT_ON_MEDIUM`: browser extras, dev, media, graphics, chat, office, gaming), no non-niri desktops, and a trimmed
`linux-firmware` (GPU, Wi-Fi, Bluetooth, sound and common Ethernet kept; `emerge -1 sys-kernel/linux-firmware` online restores the rest). The proprietary NVIDIA driver is built on the target and needs a network.
`GENTOO_OFFLINE_DIR` overrides the payload location.

## After the install

`simple-linux-update` (installed in `/usr/local/sbin`): `check` reports whether the store has a different build of the machine's kernel; `kernel` fetches it, keeps the
running one as a *previous kernel* boot entry and the newest two module trees; `system` runs `emerge --sync` and `emerge -uDN @world`; no argument does both. A machine with a built-in NVIDIA module is
refused (`--force` overrides). The store has no version index, so "newer" means "the published file differs".

**Secure Boot** is off by default (Optimization page of the GUI on UEFI; `GENTOO_INSTALLER_SECUREBOOT=1` for TUI and headless). When on, the installer makes an RSA-3072 `db` key for *this machine*
(`/etc/simple-linux/secureboot`, root only, unencrypted), signs Limine, pins the config and kernel by BLAKE2b like the live images (`iso/secureboot/README.md`), and puts `db.cer` in `/boot/secureboot/` to enrol
through the firmware menu. The update tool re-signs after a new kernel. Preflight refuses the option when `limine`, `sbsign`, `sbverify`, `b2sum` or `openssl` is missing from the live system.

## Images

`iso/` builds the live images; `iso/release.sh <version> [step…]` runs the whole recipe (build, Limine BIOS stages, rootfs, both ISOs with Secure Boot signing, offline payload and images, stage3 and kernel files, signed
`SHA256SUMS`). `check` is read-only and refuses a dirty tree, a stale binary or a missing input; pass `PROFILE=` (the saved desktop profile) or the GUI image lacks its dock and colours. It does not commit or publish. The images
boot on BIOS and UEFI, and from Ventoy (grub2 mode and, since 0.2.15, normal mode). See [iso/README.md](iso/README.md).

## Status

Verified in QEMU/KVM, not only in unit tests: full headless installs (default and all software groups, binary only), the installed disk booting to a login with the right user, shell, subvolumes and services, resume after a failed phase, the
offline GUI image installing with the outside network blocked, a headless install into the free space of a disk that holds other systems (they stayed intact,
the Limine menu listed them, the new system booted), an install that built its own kernel from the kernel matrix (booted on it), both images on BIOS and UEFI, and the GUI wizard on a virtual 3D GPU.

**Not verified:** real hardware beyond the author's machines; a picture of the *installed* desktop in a VM (the generic kernels have no virtual-GPU driver); the graphical wizard end to end after the latest changes (the free-space choice and the layouts were only looked at, not driven through an install);
an end-to-end offline install from the minimal image; the kernel build with clang or for NVIDIA; booting the other systems from the Limine menu; the Wi-Fi flow against a real `iwd`; Secure Boot of an installed system under OVMF; LUKS (not implemented).

**Known gaps:** `CommandRunner` covers shell-outs, not HTTP, so downloads are tested in a VM rather than in `cargo test`; `useradd -p` skips the password-quality check that `chpasswd` would apply; the GUI has no page for the host name and locale list
(`simple-linux` / `en_US.UTF-8` defaults, or the environment); the text installer has no free-space or layout choice (environment only); no translated installer UI.

## Packaging

`packaging/package.sh` builds `dist/gentoo-installer-<version>-linux-<arch>.tar.gz` (binaries, desktop entries, icon, metainfo, `install.sh`); `packaging/gentoo/` has a live ebuild. See [packaging/README.md](packaging/README.md).
