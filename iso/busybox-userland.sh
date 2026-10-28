#!/bin/bash
# busybox-userland.sh <rootfs>     (run as root; builds busybox once, under a minute; needs `zig` on the build host)
# Puts BusyBox in the place of GNU coreutils in a live rootfs. BusyBox is built from busybox.config next to this script (only the applets
# that replace a coreutils program, -Os) with `zig cc -target x86_64-linux-musl`, statically: one 240 KiB file that needs no libc, so it
# works even if the image's glibc is broken. Every file that sys-apps/coreutils owns and BusyBox has an applet for becomes a symlink to
# /bin/busybox. The coreutils programs BusyBox does not have (b2sum, which the installer's Secure Boot step runs, stdbuf, od, fold, ...)
# stay GNU, so nothing that worked stops working; `busybox --list` is the authority, nothing here is a hand-kept list. Run it AFTER dracut:
# the initramfs keeps the GNU tools it was built with. Other packages (util-linux, findutils, sed, grep, ...) are not touched.
# Portage still lists sys-apps/coreutils as installed, so an `emerge` of it in the live session puts the GNU files back.
# BUSYBOX_SRC=<busybox-1.36.1.tar.bz2> names the source tarball (default: Portage's distfile, else downloaded); it is checked against SHA256.
set -euo pipefail
R=$(readlink -f "${1:?rootfs}")
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
VER=1.36.1
SHA256=b8cc24c9574d809e7279c3be349795c5d5ceb6fdf19ca709f80cde50e47de314
[ "$(id -u)" = 0 ] || { echo "busybox-userland.sh: run as root" >&2; exit 1; }
command -v zig >/dev/null || { echo "busybox-userland.sh: zig is missing (it is the musl compiler)" >&2; exit 1; }

SRC=${BUSYBOX_SRC:-/var/cache/distfiles/busybox-$VER.tar.bz2}
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
[ -f "$SRC" ] || { SRC=$W/busybox-$VER.tar.bz2; curl -fsSL -o "$SRC" "https://busybox.net/downloads/busybox-$VER.tar.bz2"; }
echo "$SHA256  $SRC" | sha256sum -c --quiet - || { echo "busybox-userland.sh: $SRC does not match the pinned checksum" >&2; exit 1; }

# zig's linker has no -Map / --warn-common / --verbose, which BusyBox's final link passes
cat > "$W/zcc" <<'ZCC'
#!/bin/bash
a=(); for x in "$@"; do case $x in -Wl,--warn-common|-Wl,--verbose|-Wl,-Map,*) ;; *) a+=("$x");; esac; done
exec zig cc -target x86_64-linux-musl "${a[@]}"
ZCC
chmod +x "$W/zcc"
tar -C "$W" -xf "$SRC"
cp "$HERE/busybox.config" "$W/busybox-$VER/.config"
export ZIG_GLOBAL_CACHE_DIR=$W/zig ZIG_LOCAL_CACHE_DIR=$W/zig
( cd "$W/busybox-$VER"
  yes "" | make oldconfig >/dev/null 2>&1
  make -j"$(nproc)" CC="$W/zcc" HOSTCC=gcc AR="zig ar" STRIP=strip EXTRA_CFLAGS=-Os >"$W/build.log" 2>&1 ) || { tail -20 "$W/build.log" >&2; exit 1; }

BB=/bin/busybox
install -Dm755 "$W/busybox-$VER/busybox" "$R$BB"
mapfile -t APPLETS < <(chroot "$R" "$BB" --list)
mapfile -t OWNED < <(chroot "$R" qlist sys-apps/coreutils | grep -E '^/(usr/)?s?bin/')

n=0
declare -A SEEN
for a in "${APPLETS[@]}"; do SEEN[$a]=1; done
for f in "${OWNED[@]}"; do
    [ -n "${SEEN[$(basename "$f")]:-}" ] || continue
    [ -e "$R$f" ] || [ -L "$R$f" ] || continue
    ln -sf "$BB" "$R$f"
    n=$((n + 1))
done
echo "busybox-userland.sh: $n coreutils programs are BusyBox now; kept GNU: $(
    for f in "${OWNED[@]}"; do [ -n "${SEEN[$(basename "$f")]:-}" ] || basename "$f"; done | sort -u | tr '\n' ' ')"
