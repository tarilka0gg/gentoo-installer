#!/bin/bash
# package.sh - build a release and pack it: dist/gentoo-installer-<version>-linux-x86_64.tar.gz (+ .sha256).
# The tarball unpacks to a directory with `install.sh` (install / uninstall, PREFIX, DESTDIR) and `prefix/`.
#   packaging/package.sh            build with cargo and pack
#   SKIP_BUILD=1 packaging/package.sh   pack what target/release already holds
# For a Gentoo package use the ebuild in packaging/gentoo/ instead.
set -euo pipefail
cd "$(dirname "$0")/.."
NAME=gentoo-installer
ID=io.github.tarilka0gg.GentooInstaller
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
[ "${SKIP_BUILD:-}" = 1 ] || cargo build --release --locked

D=dist/$NAME-$VERSION
rm -rf "${D:?}"
install -Dm755 target/release/installer-cli target/release/installer-gui -t "$D/prefix/bin"
install -Dm644 "packaging/$ID.desktop" "packaging/$ID.Text.desktop" -t "$D/prefix/share/applications"
install -Dm644 "packaging/$ID.metainfo.xml" -t "$D/prefix/share/metainfo"
install -Dm644 packaging/icons/$NAME.svg -t "$D/prefix/share/icons/hicolor/scalable/apps"
install -Dm644 README.md LICENSE -t "$D/prefix/share/doc/$NAME"
install -m755 packaging/install.sh "$D/install.sh"
cat > "$D/POST-INSTALL.txt" <<'TXT'
Gentoo Installer erases the disk you choose. Start it from a live system (installer-gui, or installer-cli in a
terminal); it needs root and these tools: parted, btrfs-progs, dosfstools, pciutils, git, limine, dracut.
TXT

OUT=dist/$NAME-$VERSION-linux-$ARCH.tar.gz
tar -C dist --owner=0 --group=0 -czf "$OUT" "$NAME-$VERSION"
(cd dist && sha256sum "$(basename "$OUT")" > "$(basename "$OUT").sha256")
echo "$OUT"
