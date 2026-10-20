# Copyright 2026 Simple Linux
# Distributed under the terms of the GNU General Public License v2

EAPI=8

inherit xdg

DESCRIPTION="ProtonUp-Qt: installs and updates GE-Proton, Wine-GE and other Proton builds for Steam, Lutris and Heroic"
HOMEPAGE="https://davidotek.github.io/protonup-qt/ https://github.com/DavidoTek/ProtonUp-Qt"
SRC_URI="https://github.com/DavidoTek/ProtonUp-Qt/releases/download/v${PV}/ProtonUp-Qt-${PV}-x86_64.AppImage"
S="${WORKDIR}"

LICENSE="GPL-3"
SLOT="0"
KEYWORDS="-* ~amd64"
RESTRICT="strip mirror"
QA_PREBUILT="*"

# Python and Qt come inside the image; the graphics driver and the file dialogs are the system's.
RDEPEND="
	media-libs/libglvnd
	sys-apps/xdg-desktop-portal
"

src_unpack() {
	# an AppImage unpacks itself; the extractor is built in, no FUSE needed
	cp "${DISTDIR}/${A}" "${WORKDIR}/pupqt.AppImage" || die
	chmod +x "${WORKDIR}/pupqt.AppImage" || die
	"${WORKDIR}/pupqt.AppImage" --appimage-extract >/dev/null || die "extracting the AppImage failed"
	rm "${WORKDIR}/pupqt.AppImage" || die
}

src_install() {
	local dir=/opt/protonup-qt
	dodir "${dir}"
	cp -a squashfs-root/. "${ED}${dir}" || die
	dosym "${dir}/AppRun" /usr/bin/protonup-qt
	insinto /usr/share/applications
	newins squashfs-root/net.davidotek.pupgui2.desktop protonup-qt.desktop
	sed -i 's|^Exec=.*|Exec=protonup-qt|' "${ED}/usr/share/applications/protonup-qt.desktop" || die
	local size
	for size in 64 128 256; do
		newicon -s "${size}" "squashfs-root/usr/share/icons/hicolor/${size}x${size}/net.davidotek.pupgui2.png" net.davidotek.pupgui2.png
	done
}
