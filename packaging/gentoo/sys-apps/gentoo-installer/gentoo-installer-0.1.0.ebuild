# Copyright 2026 Gentoo Authors
# Distributed under the terms of the GNU General Public License v2

EAPI=8

CRATES="
	allocator-api2@0.2.21
	anyhow@1.0.104
	async-broadcast@0.7.2
	async-channel@2.5.0
	async-executor@1.14.0
	async-io@2.6.0
	async-lock@3.4.2
	async-process@2.5.0
	async-recursion@1.1.1
	async-signal@0.2.14
	async-task@4.7.1
	async-trait@0.1.92
	atomic-waker@1.1.2
	autocfg@1.5.1
	base64@0.22.1
	bitflags@2.13.1
	block-buffer@0.10.4
	blocking@1.6.2
	bumpalo@3.20.3
	bytes@1.12.1
	cairo-rs@0.20.12
	cairo-sys-rs@0.20.10
	cassowary@0.3.0
	castaway@0.2.4
	cc@1.4.2
	cfg-expr@0.20.8
	cfg-if@1.0.4
	cfg_aliases@0.2.2
	chacha20@0.10.1
	compact_str@0.8.2
	concurrent-queue@2.5.0
	cpufeatures@0.2.17
	cpufeatures@0.3.0
	crossbeam-utils@0.8.22
	crossterm@0.28.1
	crossterm_winapi@0.9.1
	crypto-common@0.1.7
	darling@0.24.0
	darling_core@0.24.0
	darling_macro@0.24.0
	digest@0.10.7
	displaydoc@0.2.7
	either@1.17.0
	endi@1.1.1
	enumflags2@0.7.12
	enumflags2_derive@0.7.12
	equivalent@1.0.2
	errno@0.3.14
	event-listener-strategy@0.5.4
	event-listener@5.4.2
	fastrand@2.5.0
	field-offset@0.3.6
	find-msvc-tools@0.1.10
	foldhash@0.1.5
	form_urlencoded@1.2.2
	futures-channel@0.3.34
	futures-core@0.3.34
	futures-executor@0.3.34
	futures-io@0.3.34
	futures-lite@2.6.1
	futures-macro@0.3.34
	futures-sink@0.3.34
	futures-task@0.3.34
	futures-util@0.3.34
	gdk-pixbuf-sys@0.20.10
	gdk-pixbuf@0.20.10
	gdk4-sys@0.9.6
	gdk4@0.9.6
	generic-array@0.14.7
	getrandom@0.2.17
	getrandom@0.4.3
	gio-sys@0.20.10
	gio@0.20.12
	glib-macros@0.20.12
	glib-sys@0.20.10
	glib@0.20.12
	gobject-sys@0.20.10
	graphene-rs@0.20.10
	graphene-sys@0.20.10
	gsk4-sys@0.9.6
	gsk4@0.9.6
	gtk4-macros@0.9.5
	gtk4-sys@0.9.6
	gtk4@0.9.7
	hashbrown@0.15.5
	hashbrown@0.17.1
	heck@0.5.0
	hermit-abi@0.5.2
	hex@0.4.3
	http-body-util@0.1.4
	http-body@1.1.0
	http@1.5.0
	httparse@1.10.1
	hyper-rustls@0.27.9
	hyper-util@0.1.20
	hyper@1.11.0
	icu_collections@2.2.0
	icu_locale_core@2.2.0
	icu_normalizer@2.2.0
	icu_normalizer_data@2.2.0
	icu_properties@2.2.0
	icu_properties_data@2.2.0
	icu_provider@2.2.0
	ident_case@1.0.1
	idna@1.1.0
	idna_adapter@1.2.2
	indexmap@2.14.0
	indoc@2.0.7
	instability@0.3.13
	ipnet@2.12.1
	itertools@0.13.0
	itoa@1.0.18
	js-sys@0.3.104
	libadwaita-sys@0.7.2
	libadwaita@0.7.2
	libc@0.2.189
	linux-raw-sys@0.12.1
	linux-raw-sys@0.4.15
	litemap@0.8.2
	lock_api@0.4.14
	log@0.4.33
	lru-slab@0.1.2
	lru@0.12.5
	memchr@2.8.3
	memoffset@0.9.1
	mio@1.2.2
	once_cell@1.21.4
	ordered-stream@0.2.0
	pango-sys@0.20.10
	pango@0.20.12
	parking@2.2.1
	parking_lot@0.12.5
	parking_lot_core@0.9.12
	paste@1.0.15
	percent-encoding@2.3.2
	pin-project-lite@0.2.17
	piper@0.2.5
	pkg-config@0.3.33
	polling@3.11.0
	potential_utf@0.1.5
	proc-macro-crate@3.5.0
	proc-macro2@1.0.107
	quinn-proto@0.11.16
	quinn-udp@0.5.15
	quinn@0.11.11
	quote@1.0.47
	r-efi@6.0.0
	rand@0.10.2
	rand_core@0.10.1
	rand_pcg@0.10.2
	ratatui@0.29.0
	redox_syscall@0.5.18
	reqwest@0.12.28
	ring@0.17.14
	rustc-hash@2.1.3
	rustc_version@0.4.1
	rustix@0.38.44
	rustix@1.1.4
	rustls-pki-types@1.15.1
	rustls-webpki@0.103.14
	rustls@0.23.43
	rustversion@1.0.23
	ryu@1.0.23
	scopeguard@1.2.0
	semver@1.0.28
	serde@1.0.229
	serde_core@1.0.229
	serde_derive@1.0.229
	serde_json@1.0.151
	serde_repr@0.1.21
	serde_spanned@1.1.1
	serde_urlencoded@0.7.1
	sha2@0.10.9
	shlex@2.0.1
	signal-hook-mio@0.2.5
	signal-hook-registry@1.4.8
	signal-hook@0.3.18
	slab@0.4.12
	smallvec@1.15.2
	socket2@0.6.5
	stable_deref_trait@1.2.1
	static_assertions@1.1.0
	strsim@0.11.1
	strum@0.26.3
	strum_macros@0.26.4
	subtle@2.6.1
	syn@2.0.119
	syn@3.0.3
	sync_wrapper@1.0.2
	synstructure@0.13.2
	system-deps@7.0.8
	target-lexicon@0.13.5
	tempfile@3.27.0
	thiserror-impl@2.0.20
	thiserror@2.0.20
	tinystr@0.8.3
	tinyvec@1.12.0
	tinyvec_macros@0.1.1
	tokio-macros@2.7.2
	tokio-rustls@0.26.4
	tokio-stream@0.1.19
	tokio-util@0.7.19
	tokio@1.53.1
	toml@1.1.4+spec-1.1.0
	toml_datetime@1.1.1+spec-1.1.0
	toml_edit@0.25.13+spec-1.1.0
	toml_parser@1.1.3+spec-1.1.0
	toml_writer@1.1.2+spec-1.1.0
	tower-http@0.6.11
	tower-layer@0.3.3
	tower-service@0.3.3
	tower@0.5.3
	tracing-attributes@0.1.31
	tracing-core@0.1.36
	tracing@0.1.44
	try-lock@0.2.5
	typenum@1.20.1
	uds_windows@1.2.1
	unicode-ident@1.0.24
	unicode-segmentation@1.13.3
	unicode-truncate@1.1.0
	unicode-width@0.1.14
	unicode-width@0.2.0
	untrusted@0.9.0
	url@2.5.8
	utf8_iter@1.0.4
	uuid@1.24.0
	version-compare@0.2.1
	version_check@0.9.5
	want@0.3.1
	wasi@0.11.1+wasi-snapshot-preview1
	wasm-bindgen-futures@0.4.77
	wasm-bindgen-macro-support@0.2.127
	wasm-bindgen-macro@0.2.127
	wasm-bindgen-shared@0.2.127
	wasm-bindgen@0.2.127
	wasm-streams@0.4.2
	web-sys@0.3.104
	web-time@1.1.0
	webpki-roots@1.0.9
	winapi-i686-pc-windows-gnu@0.4.0
	winapi-x86_64-pc-windows-gnu@0.4.0
	winapi@0.3.9
	windows-link@0.2.1
	windows-sys@0.52.0
	windows-sys@0.59.0
	windows-sys@0.61.2
	windows-targets@0.52.6
	windows_aarch64_gnullvm@0.52.6
	windows_aarch64_msvc@0.52.6
	windows_i686_gnu@0.52.6
	windows_i686_gnullvm@0.52.6
	windows_i686_msvc@0.52.6
	windows_x86_64_gnu@0.52.6
	windows_x86_64_gnullvm@0.52.6
	windows_x86_64_msvc@0.52.6
	winnow@1.0.4
	writeable@0.6.3
	yoke-derive@0.8.2
	yoke@0.8.3
	zbus@5.19.0
	zbus_macros@5.19.0
	zbus_names@4.3.4
	zcheapstr@1.1.0
	zerofrom-derive@0.1.7
	zerofrom@0.1.8
	zeroize@1.9.0
	zerotrie@0.2.4
	zerovec-derive@0.11.3
	zerovec@0.11.6
	zmij@1.0.23
	zvariant@5.14.0
	zvariant_derive@5.14.0
	zvariant_utils@4.0.0
"

inherit cargo desktop xdg

DESCRIPTION="Gentoo installer: btrfs layout, per-hardware kernel, Limine; GTK4 and TUI"
HOMEPAGE="https://github.com/tarilka0gg/gentoo-installer"
SRC_URI="
	https://github.com/tarilka0gg/gentoo-installer/archive/refs/tags/v${PV}.tar.gz -> ${P}.tar.gz
	${CARGO_CRATE_URIS}
"

LICENSE="GPL-2+"
# Dependent crate licenses (generated by packaging/gentoo/gen-ebuild-data.py)
LICENSE+=" Apache-2.0 Apache-2.0-with-LLVM-exceptions BSD Boost-1.0 CDLA-Permissive-2.0 ISC LGPL-2.1+ MIT Unicode-3.0 Unlicense ZLIB openssl"
SLOT="0"
KEYWORDS="~amd64"
IUSE="+gui"

# What the installer shells out to on the live system
RDEPEND="
	dev-vcs/git
	sys-apps/pciutils
	sys-block/parted
	sys-boot/limine
	sys-fs/btrfs-progs
	sys-fs/dosfstools
	sys-kernel/dracut
	gui? (
		>=gui-libs/gtk-4.0:4
		>=gui-libs/libadwaita-1.4:1
	)
"
DEPEND="
	gui? (
		>=gui-libs/gtk-4.0:4
		>=gui-libs/libadwaita-1.4:1
	)
"
BDEPEND="virtual/pkgconfig"

QA_FLAGS_IGNORED="usr/bin/installer-cli usr/bin/installer-gui"

pkg_setup() {
	rust_pkg_setup
}

src_compile() {
	cargo_src_compile -p installer-cli $(usev gui '-p installer-gui')
}

src_test() {
	cargo_src_test --workspace --
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
	einstalldocs
}

pkg_postinst() {
	xdg_pkg_postinst
	ewarn "gentoo-installer ERASES DISKS. Run it from a live system, as root."
}

pkg_postrm() {
	xdg_pkg_postrm
}
