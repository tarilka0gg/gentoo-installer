#!/bin/bash
# release.sh <version> [step...]       Prepares a Simple Linux release; publishing it on GitHub stays a manual step.
#
# Steps, in order (default: all of them):
#   check     read-only: clean trees, binaries newer than the sources, tools and inputs present. Always runs first.
#   build     cargo build --release (installer-cli, installer-gui)
#   limine    Limine's BIOS stages with the Ventoy patch (only if $LIMINE_DIR has no `limine` tool yet; REBUILD_LIMINE=1 forces)
#   rootfs    refresh-rootfs.sh for live-root-gui (gui) and live-root (minimal)       [root]
#   assemble  assemble-iso.sh for both images, Secure Boot signed                     [root]
#   collect   stage3 and kernel files, copied from the previous release
#   sign      SHA256SUMS + signature (as the signing user), verified, copied into the simple-linux repo, README version bumped
#
# `release.sh 0.2.22 check` is safe anywhere. The root steps are heavy; the rootfs step can be done on the build server by
# hand (see the memory notes) and this script resumed from `assemble`.
#
# Why the checks exist: 0.2.14 and 0.2.15 shipped a graphical installer built before the last changes, because nothing
# compared the binary in the image with the sources. `check` refuses a dirty tree or a binary older than the newest source file,
# and BUILD-INFO in the release directory records the commit and the binary hashes that went into the images.
#
# Environment (defaults in brackets):
#   T [~/gentoo-installer-test]        work area: iso/ (rootfs trees), release-<version>/, limine-ventoy/, theme-assets/
#   KEYS [~/.config/simple-linux/secureboot]   Secure Boot key set from secureboot/make-keys.sh
#   PROFILE [unset]                    make-profile.py output for the GUI image (optional)
#   WALLPAPERS [<simple-linux repo>/wallpapers]   THEME_ASSETS [$T/theme-assets]   WM_CONFIGS [sibling simple-linux-configs]
#   SIGN_USER [tarilka0gg]             who holds the release-signing key (root steps drop to this user)
set -euo pipefail

VERSION=${1:?usage: release.sh <version> [step...]}
shift || true
[[ $VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "release.sh: version must look like 0.2.22" >&2; exit 2; }
STEPS=("$@"); [ ${#STEPS[@]} -gt 0 ] || STEPS=(build limine rootfs assemble collect sign)

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
PROJECTS=$(cd "$REPO/.." && pwd)
SL_REPO=${SL_REPO:-$PROJECTS/simple-linux}
SIGN_USER=${SIGN_USER:-tarilka0gg}
SIGN_HOME=$(getent passwd "$SIGN_USER" | cut -d: -f6)
T=${T:-$SIGN_HOME/gentoo-installer-test}
W=$T/iso
OUT=$T/release-$VERSION
KEYS=${KEYS:-$SIGN_HOME/.config/simple-linux/secureboot}
LIMINE_DIR=${LIMINE_DIR:-$T/limine-ventoy}
export WM_CONFIGS=${WM_CONFIGS:-$PROJECTS/simple-linux-configs}
export WALLPAPERS=${WALLPAPERS:-$SL_REPO/wallpapers}
export THEME_ASSETS=${THEME_ASSETS:-$T/theme-assets}
BIN=$REPO/target/release

say() { printf '== %s\n' "$*"; }
die() { printf 'release.sh: %s\n' "$*" >&2; exit 1; }
as_user() {   # run a command as the signing user when we are root
    if [ "$(id -u)" = 0 ] && [ "$SIGN_USER" != root ]; then runuser -u "$SIGN_USER" -- "$@"; else "$@"; fi
}
git_user() { as_user git -C "$1" "${@:2}"; }
need_root() { [ "$(id -u)" = 0 ] || die "step '$1' needs root"; }
has_step() { local s; for s in "${STEPS[@]}"; do [ "$s" = "$1" ] && return 0; done; return 1; }

previous_release() {   # the newest release-* directory other than this one
    local d best=
    for d in "$T"/release-[0-9]*; do
        [ -d "$d" ] && [ "$d" != "$OUT" ] || continue
        best=$(printf '%s\n%s\n' "${best:-$d}" "$d" | sort -V | tail -n1)
    done
    printf '%s' "$best"
}

step_check() {
    say "check"
    local bad=0
    fail() { printf '  FAIL: %s\n' "$*"; bad=1; }
    for r in "$REPO" "$WM_CONFIGS"; do
        [ -d "$r/.git" ] || { fail "$r is not a git checkout"; continue; }
        [ -z "$(git_user "$r" status --porcelain)" ] || fail "$r has uncommitted changes (the images would not match any commit)"
    done
    for b in installer-cli installer-gui; do
        if [ ! -f "$BIN/$b" ]; then
            has_step build || fail "$BIN/$b is missing (run the build step)"
        elif ! has_step build; then
            newer=$(find "$REPO/installer-core" "$REPO/installer-cli" "$REPO/installer-gui" "$REPO/Cargo.toml" "$REPO/Cargo.lock" \
                -type f \( -name '*.rs' -o -name '*.sh' -o -name 'Cargo.*' \) -newer "$BIN/$b" -not -path '*/target/*' | head -n3)
            [ -z "$newer" ] || fail "$b is older than: $(echo "$newer" | tr '\n' ' ')"
        fi
    done
    has_step build && { command -v cargo >/dev/null 2>&1 || fail "cargo not found"; }
    if has_step rootfs || has_step assemble; then
        command -v unshare >/dev/null 2>&1 || fail "'unshare' not found"
        [ -f "$KEYS/db.key" ] || fail "Secure Boot keys not found in $KEYS (secureboot/make-keys.sh)"
        [ -d "$W/live-root-gui" ] && [ -d "$W/live-root" ] || fail "$W has no live-root / live-root-gui"
        [ -f "$W/vmlinuz-live" ] || fail "$W/vmlinuz-live is missing"
    fi
    has_step rootfs && { [ -d "$W/modroot/lib/modules" ] || fail "$W/modroot is missing (make modules_install INSTALL_MOD_PATH)"; }
    has_step sign && { [ -f "$SIGN_HOME/.config/simple-linux/release-signing" ] || fail "release signing key missing"; }
    PREV=$(previous_release)
    [ -n "$PREV" ] || fail "no earlier release-* directory to take stage3 and the kernel files from"
    [ "$bad" = 0 ] || die "check failed"
    say "check ok (previous release: ${PREV##*/})"
}

step_build() {
    say "build"
    as_user cargo build --release --manifest-path "$REPO/Cargo.toml" -p installer-cli -p installer-gui
}

step_limine() {
    if [ -x "$LIMINE_DIR/limine" ] && [ -z "${REBUILD_LIMINE:-}" ]; then say "limine: $LIMINE_DIR is ready"; return; fi
    say "limine"
    as_user "$HERE/build-limine-bios.sh" "$LIMINE_DIR"
}

step_rootfs() {
    need_root rootfs; say "rootfs (gui)"
    INSTALLER_TARGET_DIR=$BIN "$HERE/refresh-rootfs.sh" "$W" live-root-gui gui
    say "rootfs (minimal)"
    INSTALLER_TARGET_DIR=$BIN "$HERE/refresh-rootfs.sh" "$W" live-root
}

step_assemble() {
    need_root assemble; mkdir -p "$OUT"
    for pair in "live-root-gui:simple-linux-gui.iso" "live-root:simple-linux-minimal.iso"; do
        say "assemble ${pair#*:}"
        SECUREBOOT_KEYS=$KEYS LIMINE_BIOS_DIR=$LIMINE_DIR LIMINE_TOOL=$LIMINE_DIR/limine ROOTFS=${pair%%:*} \
            "$HERE/assemble-iso.sh" "$W" "$OUT/${pair#*:}"
    done
    chown -R "$SIGN_USER": "$OUT"
    {
        echo "version: $VERSION"
        echo "built: $(date -u +%FT%TZ)"
        echo "gentoo-installer: $(git_user "$REPO" rev-parse HEAD)"
        echo "simple-linux-configs: $(git_user "$WM_CONFIGS" rev-parse HEAD)"
        (cd "$BIN" && sha256sum installer-cli installer-gui)
    } > "$OUT/BUILD-INFO"
    chown "$SIGN_USER": "$OUT/BUILD-INFO"
}

step_collect() {
    say "collect"
    PREV=$(previous_release); [ -n "$PREV" ] || die "no earlier release to copy from"
    mkdir -p "$OUT"
    local f
    for f in simple-linux-stage3-amd64-openrc.tar.xz simple-linux-stage3-amd64-openrc.tar.xz.sha512 \
             simple-linux-kernel-store.tar; do
        [ -e "$PREV/$f" ] || die "$PREV/$f is missing"
        [ -e "$OUT/$f" ] || cp --reflink=auto "$PREV/$f" "$OUT/$f"
    done
    for f in "$PREV"/simple-linux-kernel-*.vmlinuz "$PREV"/simple-linux-kernel-*-modules.tar.xz; do
        [ -e "$f" ] || die "no kernel files in $PREV"
        [ -e "$OUT/${f##*/}" ] || cp --reflink=auto "$f" "$OUT/"
    done
    [ "$(id -u)" != 0 ] || chown -R "$SIGN_USER": "$OUT"
}

step_sign() {
    say "sign"
    local files=(simple-linux-minimal.iso simple-linux-gui.iso simple-linux-stage3-amd64-openrc.tar.xz)
    local k m
    k=$(cd "$OUT" && ls simple-linux-kernel-*.vmlinuz); m=$(cd "$OUT" && ls simple-linux-kernel-*-modules.tar.xz)
    files+=("$k" "$m" simple-linux-kernel-store.tar)
    for f in "${files[@]}"; do [ -f "$OUT/$f" ] || die "$OUT/$f is missing"; done
    (cd "$OUT" && as_user sh "$SL_REPO/signing/sign.sh" "${files[@]}")
    (cd "$OUT" && as_user sh "$SL_REPO/signing/verify.sh" .) || die "the fresh signature does not verify"
    cp "$OUT/SHA256SUMS" "$OUT/SHA256SUMS.sig" "$SL_REPO/"
    as_user sed -i -E "s/(Current version: \*\*)[0-9.]+(\*\*)/\1$VERSION\2/" "$SL_REPO/README.md"
    say "signed. Still to do by hand: a 'What $VERSION changed' section in $SL_REPO/README.md, the commit, then publish"
    echo "   files to attach: $OUT/*"
}

step_check
for s in "${STEPS[@]}"; do
    case $s in
        check) ;;
        build|limine|rootfs|assemble|collect|sign) "step_$s" ;;
        *) die "unknown step '$s'" ;;
    esac
done
