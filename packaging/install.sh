#!/bin/sh
# gentoo-installer installer (shipped inside the release tarball).
#
#   ./install.sh                 install under /usr/local (needs root for that prefix)
#   PREFIX=$HOME/.local ./install.sh   per-user install, no root; /etc files are skipped
#   DESTDIR=/tmp/stage ./install.sh    stage into a directory (for packagers)
#   ./install.sh uninstall       remove what this script installed (uses the recorded manifest)
#
# The tarball holds `prefix/` (goes under PREFIX) and, optionally, `etc/` (goes to /etc; existing config files
# are never overwritten, the new copy is written next to them as `<name>.new`).
set -eu
NAME=gentoo-installer
PREFIX=${PREFIX:-/usr/local}
DESTDIR=${DESTDIR:-}
HERE=$(cd "$(dirname "$0")" && pwd)
MANIFEST=$DESTDIR$PREFIX/share/$NAME/manifest

say() { printf '%s\n' "$*"; }

install_tree() { # <src dir> <dst dir> <config?>
    [ -d "$1" ] || return 0
    (cd "$1" && find . -type f | sort) | while IFS= read -r f; do
        f=${f#./}; dst=$2/$f
        mode=$(stat -c %a "$1/$f")
        if [ "${3:-}" = config ] && [ -e "$dst" ]; then
            install -Dm"$mode" "$1/$f" "$dst.new"; say "kept existing $dst (new copy: $dst.new)"
            echo "$dst.new" >> "$MANIFEST"; continue
        fi
        install -Dm"$mode" "$1/$f" "$dst"; echo "$dst" >> "$MANIFEST"
    done
}

refresh_caches() {
    [ -z "$DESTDIR" ] || return 0
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
}

case "${1:-install}" in
install)
    mkdir -p "$(dirname "$MANIFEST")"; : > "$MANIFEST"
    echo "$MANIFEST" >> "$MANIFEST"
    install_tree "$HERE/prefix" "$DESTDIR$PREFIX"
    if [ -d "$HERE/etc" ]; then
        if [ -n "$DESTDIR" ] || [ "$(id -u)" = 0 ]; then
            install_tree "$HERE/etc" "$DESTDIR/etc" config
        else
            say "not root: skipping /etc files (services, udev rules); run as root to install them"
        fi
    fi
    refresh_caches
    say "$NAME installed under $DESTDIR$PREFIX"
    [ ! -f "$HERE/POST-INSTALL.txt" ] || cat "$HERE/POST-INSTALL.txt"
    ;;
uninstall)
    [ -f "$MANIFEST" ] || { say "no manifest at $MANIFEST: nothing recorded to remove" >&2; exit 1; }
    while IFS= read -r f; do rm -f -- "$f"; d=$(dirname "$f"); rmdir -p --ignore-fail-on-non-empty "$d" 2>/dev/null || true; done < "$MANIFEST"
    refresh_caches
    say "$NAME removed"
    ;;
*) say "usage: $0 [install|uninstall]" >&2; exit 2 ;;
esac
