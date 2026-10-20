# Copyright 2026 Simple Linux
# Distributed under the terms of the GNU General Public License v2

EAPI=8

DESCRIPTION="Kernel and login settings that Steam, Proton and most games expect (Simple Linux)"
HOMEPAGE="https://github.com/tarilka0gg/gentoo-installer"
S="${WORKDIR}"

LICENSE="GPL-2"
SLOT="0"
KEYWORDS="~amd64"

RDEPEND="
	sys-process/procps
	sys-libs/pam
"

src_install() {
	# Many games (and Proton's wine) map far more memory areas than the kernel's default 65530 allows; they crash without this.
	insinto /etc/sysctl.d
	newins - 90-simple-linux-gaming.conf <<-'CONF'
	# Steam Deck's value. Games such as Hogwarts Legacy, DayZ and Star Citizen need more than the default of 65530.
	vm.max_map_count = 2147483642
	CONF

	# /dev/uinput is what Steam Input uses for virtual controllers; /dev/ntsync is what Proton uses for fast Windows synchronisation.
	# Neither module loads by itself.
	insinto /etc/modules-load.d
	newins - simple-linux-gaming.conf <<-'CONF'
	uinput
	ntsync
	CONF

	# Proton's esync needs a hard limit of open files of 524288; GameMode asks for a lower nice value and a real-time priority.
	# On a one-user desktop the user is in `wheel`, which is where GameMode's own `gamemode` group rule is repeated.
	insinto /etc/security/limits.d
	newins - 90-simple-linux-gaming.conf <<-'CONF'
	*       hard    nofile  524288
	@wheel  -       nice    -10
	@wheel  -       rtprio  10
	CONF

	# access to /dev/uinput and /dev/ntsync for the logged-in user (the rules of game-device-udev-rules cover the controllers)
	insinto /lib/udev/rules.d
	newins - 70-simple-linux-gaming.rules <<-'RULES'
	KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"
	KERNEL=="ntsync", SUBSYSTEM=="misc", TAG+="uaccess", MODE="0666"
	RULES
}
