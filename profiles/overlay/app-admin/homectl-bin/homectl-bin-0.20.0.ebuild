# Copyright 2026 Simple Linux
# Distributed under the terms of the GNU General Public License v2

EAPI=8

DESCRIPTION="Control this machine from your phone: monitor, terminal, files, power, lights (prebuilt)"
HOMEPAGE="https://github.com/tarilka0gg/homectl"
BASE="https://github.com/tarilka0gg/homectl/releases/download/v${PV}"
RAW="https://raw.githubusercontent.com/tarilka0gg/homectl/v${PV}/packaging"
SRC_URI="
	${BASE}/homectl-linux-x86_64 -> ${P}-linux-x86_64
	${BASE}/homectl-pty -> ${P}-pty
	${BASE}/homectl-adopt -> ${P}-adopt
	${BASE}/homectl-pair-linux-x86_64 -> ${P}-pair
	${RAW}/homectl.initd -> ${P}.initd
	${RAW}/homectl-launch.sh -> ${P}-launch.sh
	${RAW}/homectl-pair.desktop -> ${P}-pair.desktop
"
S="${WORKDIR}"

LICENSE="GPL-2"
SLOT="0"
KEYWORDS="-* ~amd64"
RESTRICT="strip mirror"
QA_PREBUILT="*"

RDEPEND="
	sys-apps/openrc
	gui-libs/gtk:4
	media-gfx/qrencode
"

src_install() {
	newbin "${DISTDIR}/${P}-linux-x86_64" homectl
	newbin "${DISTDIR}/${P}-pty" homectl-pty
	newbin "${DISTDIR}/${P}-adopt" homectl-adopt
	newbin "${DISTDIR}/${P}-launch.sh" homectl-launch
	newbin "${DISTDIR}/${P}-pair" homectl-pair

	# the window that shows the pairing code and link, in the application menu
	insinto /usr/share/applications
	newins "${DISTDIR}/${P}-pair.desktop" homectl-pair.desktop

	# the init script must find the binaries where this package installs them
	# the service user defaults to the first login account (uid 1000), which the installer creates
	sed -e 's#/usr/local/bin/#/usr/bin/#g' \
		-e 's#: "${HOMECTL_USER:=root}"#: "${HOMECTL_USER:=$(getent passwd 1000 | cut -d: -f1)}"#' \
		"${DISTDIR}/${P}.initd" > "${T}/homectl.initd" || die
	grep -q 'getent passwd 1000' "${T}/homectl.initd" || die "initd default user not patched"
	newinitd "${T}/homectl.initd" homectl

	# the tunnel stays off by default, so nothing is exposed until the user turns it on
	printf '%s\n' \
		'# HOMECTL_USER defaults to the first login account; set it to run the daemon as another user.' \
		'HOMECTL_BIN=/usr/bin/homectl' \
		'# off: no public tunnel; auto or quick: see the homectl README' \
		'HOMECTL_TUNNEL=off' > "${T}/homectl.confd" || die
	newconfd "${T}/homectl.confd" homectl
}
