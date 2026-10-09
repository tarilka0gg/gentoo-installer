#!/bin/bash
# build.sh <work-dir> <world-file> [--jobs N] [--unpack-only]    (run as root, on the laptop or on the build server)
#
# Builds the binary packages the installer would otherwise compile: the Portage packages that Gentoo's own binhost does not
# have (niri, noctalia, fish, micro, ...) and those whose USE differs from it. A stage3 of the project is unpacked into
# <work-dir>/root, given the pinned Portage tree and GURU snapshot, the installer's own Portage configuration (spec/), a *generic*
# x86-64 build configuration (a binary must run on every CPU: no -march=native), and `emerge --buildpkg` is run for the atoms in
# <world-file>. Packages come from Gentoo's binhost where they match and are compiled only where they do not. What was compiled
# ends up, GPG-signed, in <work-dir>/root/var/cache/binpkgs with a Packages index.
#
# Inputs, all under $T (default ~/gentoo-installer-test/binhost):
#   stage3.tar.xz      the project's stage3 (release asset simple-linux-stage3-amd64-openrc.tar.xz)
#   tree-*.tar.zst     the Portage tree + GURU snapshot (gentoo/ and guru/); the same snapshot on every machine
# Outputs: <work-dir>/out/{binpkgs/, binhost-signing.asc (public key), build.log}
#
# Cores: this script does not pin cores. On the laptop run it as `taskset -c 0-15 nice -n 10 build.sh ...` (P cores only).
# Mounts live in a private namespace (unshare), nothing outlives the script. The work directory may be reused: an
# existing root/ is kept and only refreshed, and packages already in binpkgs/ are reused by --usepkg.
set -euo pipefail
W=$(mkdir -p "${1:?work dir}" && readlink -f "$1"); WORLD=$(readlink -f "${2:?world file}"); shift 2
JOBS=$(nproc); UNPACK_ONLY=
while [ $# -gt 0 ]; do case $1 in --jobs) JOBS=$2; shift 2 ;; --unpack-only) UNPACK_ONLY=1; shift ;; *) echo "build.sh: unknown $1" >&2; exit 2 ;; esac; done
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=${T:-$HOME/gentoo-installer-test/binhost}
STAGE3=${STAGE3:-$T/stage3.tar.xz}
TREE=$(ls -1 "$T"/tree-*.tar.zst 2>/dev/null | sort | tail -n1 || true)
[ "$(id -u)" = 0 ] || { echo "build.sh: run as root" >&2; exit 1; }
R=$W/root; OUT=$W/out; mkdir -p "$OUT"

if [ ! -e "$R/.stage-unpacked" ]; then
    [ -f "$STAGE3" ] || { echo "build.sh: $STAGE3 is missing" >&2; exit 1; }
    echo "== stage3"; mkdir -p "$R"
    tar --numeric-owner --xattrs-include='*.*' -xpf "$STAGE3" -C "$R"
    touch "$R/.stage-unpacked"
fi
if [ -n "$TREE" ] && [ ! -e "$R/.tree-$(basename "$TREE")" ]; then
    echo "== Portage tree and GURU: $(basename "$TREE")"
    rm -rf "$R/var/db/repos/gentoo" "$R/var/db/repos/guru"
    mkdir -p "$R/var/db/repos"; tar --zstd -xpf "$TREE" -C "$R/var/db/repos"
    rm -f "$R"/.tree-*; touch "$R/.tree-$(basename "$TREE")"
fi

[ -z "$UNPACK_ONLY" ] || { echo "unpacked in $R"; exit 0; }
ls "$R"/.tree-* >/dev/null 2>&1 || { echo "build.sh: no Portage tree in $R" >&2; exit 1; }
echo "== configuration"
P=$R/etc/portage; SPEC=$HERE/spec/etc-portage
for d in package.use package.accept_keywords package.mask package.license profile; do
    rm -rf "${P:?}/$d"; [ -e "$SPEC/$d" ] && cp -a "$SPEC/$d" "$P/$d"
done
mkdir -p "$P/repos.conf" && cp "$SPEC/guru.repos.conf" "$P/repos.conf/guru.conf"
# make.conf: the stage3's own, minus what we own, plus a generic build (same keys the installer owns, other values).
sed -i -E '/^(COMMON_FLAGS|CFLAGS|CXXFLAGS|MAKEOPTS|CPU_FLAGS_X86|VIDEO_CARDS|FEATURES|PKGDIR|BINPKG_FORMAT|EMERGE_DEFAULT_OPTS|BINPKG_GPG_[A-Z_]*)=/d; /^# Binhost build/d' "$P/make.conf"
GNUPG=$W/gnupg-sign
if [ ! -d "$GNUPG" ]; then
    mkdir -m 700 "$GNUPG"
    gpg --homedir "$GNUPG" --batch --passphrase '' --quick-generate-key "Simple Linux binhost <binhost@localhost>" ed25519 sign never
fi
FPR=$(gpg --homedir "$GNUPG" --batch --list-secret-keys --with-colons | awk -F: '/^fpr/{print $10; exit}')
gpg --homedir "$GNUPG" --armor --export "$FPR" > "$OUT/binhost-signing.asc"
mkdir -p "$R/root/gnupg-sign"; cp -a "$GNUPG/." "$R/root/gnupg-sign/"
cat >> "$P/make.conf" <<MAKECONF
# Binhost build (iso/binhost/build.sh): generic x86-64, as Gentoo's own binhost, so the packages run on any CPU.
COMMON_FLAGS="-O2 -pipe"
CFLAGS="\${COMMON_FLAGS}"
CXXFLAGS="\${COMMON_FLAGS}"
MAKEOPTS="-j$JOBS"
VIDEO_CARDS="amdgpu intel nouveau radeon radeonsi"
FEATURES="getbinpkg buildpkg binpkg-signing binpkg-ignore-signature parallel-install"
BINPKG_FORMAT="gpkg"
PKGDIR="/var/cache/binpkgs"
BINPKG_GPG_SIGNING_GPG_HOME="/root/gnupg-sign"
BINPKG_GPG_SIGNING_KEY="$FPR"
EMERGE_DEFAULT_OPTS="--jobs=2 --load-average=$JOBS"
MAKECONF
cp "$WORLD" "$R/root/world-to-build"; cp "$OUT/binhost-signing.asc" "$R/root/"; echo "$FPR" > "$R/root/binhost-fpr"
cat > "$R/root/build-inner.sh" <<'INNER'
set -e
export LC_ALL=C.UTF-8
mkdir -p /run/lock; chmod 755 /run/lock     # binpkg-signing takes a lock there
# The build chroot does not verify signatures of what it fetches (the official binhost, over https) or re-reads (its own output):
# gpg under Portage's unprivileged fetch user could not write its home in this chroot. The packages it *publishes* are signed.
sed -i 's/^verify-signature *=.*/verify-signature = false/' /etc/portage/binrepos.conf/*.conf 2>/dev/null || true
atoms=$(grep -v '^#' /root/world-to-build | tr '\n' ' ')
echo "== emerge: $atoms"
# Same as the installer: USE and keyword changes the dependencies need are written to /etc/portage (CONFIG_PROTECT_MASK keeps
# them out of ._cfg files). They are collected into spec/ afterwards, so the installer asks for exactly these flags.
# A failed download (mirrors come and go) should cost one retry, not the whole build: what was built is reused (--usepkg).
for attempt in 1 2 3; do
    CONFIG_PROTECT_MASK=/etc/portage emerge -v --noreplace --getbinpkg --usepkg --buildpkg --keep-going --autounmask-write --autounmask-continue $atoms && break
    [ "$attempt" = 3 ] && exit 1
    echo "== emerge failed, attempt $attempt of 3: trying again"; sleep 20
done
echo "== index"
emaint binhost --fix >/dev/null 2>&1 || true
echo "== done"; ls /var/cache/binpkgs | head -50
INNER

cat > "$W/ns.sh" <<NS
set -e
mount --bind /proc "$R/proc"; mount --rbind /sys "$R/sys"; mount --rbind /dev "$R/dev"
# Build directories in RAM: fast, and gone afterwards. BUILD_TMPFS=0 keeps them on disk (the 8 GB build server ran out of memory with it).
[ "${BUILD_TMPFS:-1}" = 0 ] || mount -t tmpfs -o size=6g tmpfs "$R/var/tmp"
# The host's resolver may be a private one (tailscale on the build server) that fails now and then: the chroot gets public ones.
printf "nameserver 1.1.1.1\nnameserver 8.8.8.8\n" > "$R/etc/resolv.conf"
# ... and IPv4 first: on the build server an IPv6 address is returned that goes nowhere, and every download waited for its timeout.
printf "precedence ::ffff:0:0/96  100\n" > "$R/etc/gai.conf"
exec chroot "$R" /bin/bash /root/build-inner.sh
NS
echo "== build ($JOBS jobs) — log: $OUT/build.log"
unshare --mount --propagation private bash "$W/ns.sh" 2>&1 | tee "$OUT/build.log"
: > "$R/etc/resolv.conf"
rm -rf "$OUT/binpkgs" "$OUT/etc-portage"; cp -a "$R/var/cache/binpkgs" "$OUT/binpkgs"
mkdir -p "$OUT/etc-portage"; for d in package.use package.accept_keywords package.license; do cp -a "$R/etc/portage/$d" "$OUT/etc-portage/$d" 2>/dev/null || true; done
echo "binary packages in $OUT/binpkgs"
