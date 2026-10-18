# Copyright 2026 Simple Linux
# Distributed under the terms of the GNU General Public License v2

EAPI=8

inherit xdg

DESCRIPTION="Page and layout design editor (DesignCraft, clean-room, in Rust)"
HOMEPAGE="https://getartcraft.com/apps/designcraft https://github.com/storytold/designcraft"
SRC_URI="https://github.com/storytold/designcraft/releases/download/v${PV}/designcraft-${PV}-linux-x86_64.tar.gz"
S="${WORKDIR}/designcraft-${PV}-linux-x86_64"

LICENSE="|| ( Apache-2.0 MIT ) OFL-1.1"
SLOT="0"
KEYWORDS="-* ~amd64"
RESTRICT="strip mirror"
QA_PREBUILT="*"

# The program is one static-ish binary: it loads the window system and the graphics drivers at run time.
RDEPEND="
	dev-libs/wayland
	media-libs/libglvnd
	media-libs/vulkan-loader
	sys-apps/dbus
	x11-libs/libX11
	x11-libs/libXcursor
	x11-libs/libXi
	x11-libs/libxcb
	x11-libs/libxkbcommon
"

src_install() {
	dobin bin/*
	# the documents go to the package's own directory, the rest to /usr/share
	local d
	for d in share/*; do
		[[ ${d} == share/doc ]] && continue
		insinto /usr/share
		doins -r "${d}"
	done
	dodoc -r share/doc/*/.
}
