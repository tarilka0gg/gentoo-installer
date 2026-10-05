# Copyright 1999-2026 Gentoo Authors
# Distributed under the terms of the GNU General Public License v2

EAPI=8

CRATES=""
# Live ebuild: cargo_live_src_unpack fetches the crates at unpack time (needs network, allowed for 9999).
# For a versioned ebuild, generate CRATES from Cargo.lock with pycargoebuild.

inherit cargo git-r3 xdg

DESCRIPTION="Gentoo installer: btrfs layout, per-hardware kernel, Limine; GTK4 and text frontends"
HOMEPAGE="https://github.com/tarilka0gg/gentoo-installer"
EGIT_REPO_URI="https://github.com/tarilka0gg/gentoo-installer.git"

LICENSE="GPL-2+"
SLOT="0"
KEYWORDS=""
IUSE="+gui"

# What the installer shells out to on the live system (see iso/README.md, step 3).
RDEPEND="
	dev-vcs/git
	sys-apps/pciutils
	sys-block/parted
	sys-boot/limine
	sys-fs/btrfs-progs
	sys-fs/dosfstools
	sys-kernel/dracut
	gui? (
		gui-libs/gtk:4
		gui-libs/libadwaita:1
	)
"
DEPEND="${RDEPEND}"
BDEPEND="virtual/pkgconfig"

QA_FLAGS_IGNORED="usr/bin/installer-cli usr/bin/installer-gui"

src_unpack() {
	git-r3_src_unpack
	cargo_live_src_unpack
}

src_compile() {
	cargo_src_compile -p installer-cli $(usev gui '-p installer-gui')
}

src_install() {
	cargo_src_install --path installer-cli
	if use gui; then
		cargo_src_install --path installer-gui
		domenu packaging/io.github.tarilka0gg.GentooInstaller.desktop
	fi
	domenu packaging/io.github.tarilka0gg.GentooInstaller.Text.desktop
	insinto /usr/share/metainfo
	doins packaging/io.github.tarilka0gg.GentooInstaller.metainfo.xml
	insinto /usr/share/icons/hicolor/scalable/apps
	doins packaging/icons/gentoo-installer.svg
	dodoc README.md
}
