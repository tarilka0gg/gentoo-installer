# Copyright 2026 Simple Linux
# Distributed under the terms of the GNU General Public License v2

EAPI=8

inherit xdg

DESCRIPTION="PineconeMC (ElyPrism Launcher): Prism Launcher with built-in Ely.by accounts, ready-made Qt6 build"
HOMEPAGE="https://pineconemc.ru https://github.com/ElyPrismLauncher/Launcher"
SRC_URI="https://github.com/ElyPrismLauncher/Launcher/releases/download/${PV}/PineconeMC-Linux-Qt6-Portable-${PV}.tar.gz"
S="${WORKDIR}/pineconemc"

LICENSE="GPL-3 LGPL-3 LGPL-2.1 Apache-2.0 MIT"
SLOT="0"
KEYWORDS="-* ~amd64"
RESTRICT="strip mirror"
QA_PREBUILT="*"

# The Qt libraries come with the program (one `sharun` bundle); the graphics driver, Java and the file dialogs are the system's.
RDEPEND="
	sys-apps/which
	virtual/jre:21
	media-libs/libglvnd
	media-libs/vulkan-loader
	sys-apps/xdg-desktop-portal
"

src_unpack() {
	# the archive has no top directory
	mkdir "${S}" || die
	cd "${S}" || die
	unpack "${A}"
}

src_install() {
	local dir=/opt/pineconemc
	# without portable.txt the launcher keeps its data in ~/.local/share, not next to the program
	rm -f portable.txt manifest.txt || die
	# the bundle keeps its own permissions and links (sharun starts the bundled loader), so it is copied, not doins
	dodir "${dir}"
	cp -a . "${ED}${dir}" || die
	dosym "${dir}/ElyPrismLauncher" /usr/bin/elyprismlauncher
	dosym elyprismlauncher /usr/bin/pineconemc
	insinto /usr/share
	doins -r share/applications share/icons share/metainfo share/mime
	doman share/man/man6/elyprismlauncher.6
}
