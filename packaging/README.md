# Packaging

| What | How |
|---|---|
| Release tarball (binaries, desktop entries, icon, metainfo, `install.sh`) | `packaging/package.sh` → `dist/gentoo-installer-<version>-linux-x86_64.tar.gz` + `.sha256` |
| Install from the tarball | `./install.sh` (under `/usr/local`), `PREFIX=$HOME/.local ./install.sh`, `DESTDIR=… ./install.sh`, `./install.sh uninstall` |
| Gentoo package | `packaging/gentoo/sys-apps/gentoo-installer/` — a live (`9999`) ebuild, USE `gui` |
| Desktop entries | `io.github.tarilka0gg.GentooInstaller.desktop` (GTK window), `….Text.desktop` (terminal) |
| AppStream | `io.github.tarilka0gg.GentooInstaller.metainfo.xml` |
| Icon | `icons/gentoo-installer.svg` (a placeholder glyph) |

## Using the ebuild

Copy `packaging/gentoo/sys-apps` into any overlay (or add this directory as one: it needs `profiles/repo_name` and
`metadata/layout.conf`), run `ebuild gentoo-installer-9999.ebuild manifest`, and unmask the live version
(`sys-apps/gentoo-installer **` in `package.accept_keywords`). A versioned ebuild needs `CRATES` generated from
`Cargo.lock` (`pycargoebuild`); the live one fetches crates at unpack time.

Checked: `desktop-file-validate`, `appstreamcli validate`, the tarball installed into a `DESTDIR` and removed again,
and `emerge -pv` resolving the ebuild's dependencies. The ebuild itself has not been built.

The installer erases disks; the GUI entry starts `installer-gui` as the current user, which only works as root
(or after the live system's own login). Run it from a live system.
